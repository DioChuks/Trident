# Kubernetes Deployment

This guide covers deploying Trident self-hosted on Kubernetes using the official Helm chart.

## Prerequisites

| Requirement | Minimum version | Notes |
|-------------|----------------|-------|
| Kubernetes | 1.25+ | EKS, GKE, AKS, or self-hosted |
| Helm | 3.12+ | `brew install helm` |
| PostgreSQL | 15+ | Operator-managed (e.g. CloudNativePG) or managed service |
| Redis | 7+ | Operator-managed (e.g. Redis Operator) or managed service |

Trident's Helm chart packages the four stateless services only — it does **not** bundle Postgres or Redis. Provision those separately before installing the chart.

## Quick Start

### 1. Add the chart repository (once published)

```bash
helm repo add trident https://telocel-labs.github.io/trident
helm repo update
```

For now, install directly from the cloned repo:

```bash
git clone https://github.com/telocel-labs/trident
cd trident
```

### 2. Create the secrets

Trident uses the `existingSecret` pattern — sensitive values are read from a Kubernetes Secret rather than passed through Helm values.

```bash
kubectl create secret generic trident-secrets \
  --from-literal=DATABASE_URL="postgres://trident:password@postgres-host:5432/trident" \
  --from-literal=REDIS_URL="redis://redis-host:6379" \
  --from-literal=ADMIN_API_KEY="$(openssl rand -hex 32)"
```

### 3. Install the chart

```bash
helm install trident ./helm/trident \
  --namespace trident \
  --create-namespace \
  --set goApi.image.tag=v0.1.0 \
  --set indexer.image.tag=v0.1.0 \
  --set grpcApi.image.tag=v0.1.0
```

### 4. Verify the deployment

```bash
kubectl -n trident get pods
kubectl -n trident get hpa
```

Expected output:

```
NAME                                    READY   STATUS    RESTARTS   AGE
trident-go-api-7d9f8c4b5-abcde         1/1     Running   0          2m
trident-go-api-7d9f8c4b5-fghij         1/1     Running   0          2m
trident-grpc-api-6c8b7d5f4-klmno       1/1     Running   0          2m
trident-indexer-5b4d9c3a2-pqrst        1/1     Running   0          2m
trident-nginx-4a3c8b7d6-uvwxy          1/1     Running   0          2m
```

## Configuration

### Using an Ingress controller instead of Nginx

Disable the bundled Nginx deployment and enable the Ingress resource:

```yaml
# custom-values.yaml
nginx:
  enabled: false

ingress:
  enabled: true
  className: nginx
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
  host: api.trident.example.com
  tls:
    - secretName: trident-tls
      hosts:
        - api.trident.example.com
```

```bash
helm upgrade trident ./helm/trident -f custom-values.yaml
```

### Horizontal Pod Autoscaler

The Go API scales automatically between 2 and 10 replicas based on CPU utilisation (target: 70%). Configure via:

```yaml
goApi:
  hpa:
    minReplicas: 2
    maxReplicas: 10
    targetCPUUtilizationPercentage: 70
```

Ensure the [Metrics Server](https://github.com/kubernetes-sigs/metrics-server) is installed in your cluster for HPA to function:

```bash
kubectl apply -f https://github.com/kubernetes-sigs/metrics-server/releases/latest/download/components.yaml
```

### Resource requests and limits

Adjust per your cluster capacity. Default values are conservative for development:

```yaml
goApi:
  resources:
    requests:
      cpu: "100m"
      memory: "128Mi"
    limits:
      cpu: "500m"
      memory: "256Mi"
```

### Multiple API key support

Create API keys via the admin endpoint after deployment:

```bash
ADMIN_KEY=$(kubectl get secret trident-secrets -o jsonpath='{.data.ADMIN_API_KEY}' | base64 -d)
TRIDENT_HOST="http://$(kubectl get svc trident-nginx -o jsonpath='{.status.loadBalancer.ingress[0].ip}')"

curl -X POST "$TRIDENT_HOST/v1/api-keys" \
  -H "X-Admin-Key: $ADMIN_KEY" \
  -H "Content-Type: application/json" \
  -d '{"label": "my-app", "network": "mainnet"}'
```

## Secrets management {#secrets}

Every deployment reads `DATABASE_URL`, `REDIS_URL`, and `ADMIN_API_KEY` from a
single Kubernetes Secret named by `global.existingSecret` (default
`trident-secrets`) via `secretKeyRef` — never from `values.yaml`, and never
`COPY`'d into an image layer (see [crates/api/Dockerfile](../crates/api/Dockerfile),
[crates/indexer/Dockerfile](../crates/indexer/Dockerfile), and
[services/api/Dockerfile](../services/api/Dockerfile): each only copies the
compiled binary out of its builder stage — no `.env` file, no secret, is ever
part of an image layer). How that one Secret gets *populated* is a separate
choice, with three supported options:

### Option 1 — `kubectl create secret` (quick start / dev)

What the Quick Start above uses. Simplest option, but the plaintext value
passes through your shell history and the `kubectl` process — fine for a
local kind cluster, not recommended for production.

### Option 2 — external-secrets operator (recommended for production)

Syncs the Secret from a real secrets backend (Vault, AWS Secrets Manager, GCP
Secret Manager, Azure Key Vault, ...) on a refresh interval, so no plaintext
value is ever typed into `kubectl` or committed anywhere.

1. Install the [external-secrets operator](https://external-secrets.io/latest/introduction/getting-started/) into the cluster (once, cluster-wide).
2. Create a `SecretStore` or `ClusterSecretStore` pointing at your backend — see the
   [external-secrets provider docs](https://external-secrets.io/latest/provider/aws-secrets-manager/)
   for backend-specific examples. Example for AWS Secrets Manager:

   ```yaml
   apiVersion: external-secrets.io/v1beta1
   kind: ClusterSecretStore
   metadata:
     name: trident-secret-store
   spec:
     provider:
       aws:
         service: SecretsManager
         region: us-east-1
         auth:
           jwt:
             serviceAccountRef:
               name: trident-external-secrets
   ```

3. Enable the chart's `ExternalSecret` and point it at that store:

   ```bash
   helm upgrade trident ./helm/trident \
     --set global.externalSecret.enabled=true \
     --set global.externalSecret.secretStoreRef.name=trident-secret-store \
     --set global.externalSecret.secretStoreRef.kind=ClusterSecretStore
   ```

   By default this expects a single backend secret at `trident/prod` with
   `DATABASE_URL`/`REDIS_URL`/`ADMIN_API_KEY` keys — override
   `global.externalSecret.data[].remoteRef` per key if your backend layout
   differs (see `helm/trident/values.yaml`).

The operator owns and continuously syncs a Secret named
`global.existingSecret` — every other deployment keeps reading it exactly the
same way, so there's no chart change needed anywhere else.

### Option 3 — Secrets Store CSI Driver

An alternative to the external-secrets operator: mount the backend secret as
a volume via the [Secrets Store CSI Driver](https://secrets-store-csi-driver.sigs.k8s.io/)
and its provider for your backend (e.g.
[aws-secrets-store-csi-driver-provider](https://github.com/aws/secrets-store-csi-driver-provider-aws),
[secrets-store-csi-driver-provider-gcp](https://github.com/GoogleCloudPlatform/secrets-store-csi-driver-provider-gcp),
[secrets-store-csi-driver-provider-azure](https://github.com/Azure/secrets-store-csi-driver-provider-azure)).
Not templated directly in this chart — the CSI driver/provider combination is
cluster- and backend-specific — but the driver's `secretObjects` field can
sync the mounted secret into a native Kubernetes Secret with the same name
(`global.existingSecret`) and keys this chart expects, so no other chart
changes are needed either. Example `SecretProviderClass`:

```yaml
apiVersion: secrets-store.csi.x-k8s.io/v1
kind: SecretProviderClass
metadata:
  name: trident-secrets-csi
spec:
  provider: aws  # or gcp / azure — matches your installed CSI provider
  parameters:
    objects: |
      - objectName: "trident/prod/DATABASE_URL"
        objectType: "secretsmanager"
      - objectName: "trident/prod/REDIS_URL"
        objectType: "secretsmanager"
      - objectName: "trident/prod/ADMIN_API_KEY"
        objectType: "secretsmanager"
  secretObjects:
    - secretName: trident-secrets   # global.existingSecret
      type: Opaque
      data:
        - objectName: "trident/prod/DATABASE_URL"
          key: DATABASE_URL
        - objectName: "trident/prod/REDIS_URL"
          key: REDIS_URL
        - objectName: "trident/prod/ADMIN_API_KEY"
          key: ADMIN_API_KEY
```

Then mount the CSI volume on at least one pod referencing this
`SecretProviderClass` (a single mount is enough to trigger the sync — the
resulting `trident-secrets` Secret is then available cluster-wide via
`secretKeyRef` exactly as with the other two options).

### Verifying no secret ends up in an image layer

```bash
docker history --no-trunc ghcr.io/telocel-labs/trident-go-api:latest | grep -i -E "DATABASE_URL|REDIS_URL|ADMIN_API_KEY|secret"
```

Should print nothing. Each Dockerfile's runtime stage only ever `COPY
--from=builder` the compiled binary — no `ENV`, no `ARG`, no `COPY` of a
secret or `.env` file appears in any layer.

### Verifying no secret is ever logged

The Go API, gRPC API, and indexer all read `DATABASE_URL`/`REDIS_URL` only to
open a connection at startup — none of the three log the connection string
itself (only connection *success/failure*, without the credential-bearing
URL). `ADMIN_API_KEY` is compared, never logged. If you add a log line near
any of these, redact the credential portion — don't log the raw env var value.

### Rotating secrets

1. Update the value in your backend (Vault/Secrets Manager/etc., or `kubectl create secret --dry-run=client -o yaml | kubectl apply -f -` for the manual path).
2. **external-secrets**: happens automatically on the next `refreshInterval` tick (default `1h` in this chart) — no manual step. To force it immediately: `kubectl annotate externalsecret trident-secrets force-sync=$(date +%s) --overwrite`.
3. **CSI**: re-mount (pod restart) picks up the new value; `secretObjects` sync depends on your provider's rotation reconciler — check its docs for a reconciliation interval.
4. **Manual `kubectl create secret`**: re-run the command with the new value, or `kubectl create secret generic trident-secrets --from-literal=... --dry-run=client -o yaml | kubectl apply -f -`.
5. **After any of the above**, roll the consuming pods so they pick up the new value — none of the three services currently hot-reload env vars:
   ```bash
   kubectl rollout restart deployment/trident-go-api deployment/trident-grpc-api deployment/trident-indexer
   ```
   (A future improvement would be [Reloader](https://github.com/stakater/Reloader) to automate this step.)

## Health checks

The Go API exposes `GET /v1/health`. Kubernetes liveness and readiness probes are pre-configured in the chart:

- **Liveness** (`failureThreshold: 3`): restarts the container after 3 consecutive failures.
- **Readiness** (`failureThreshold: 1`): removes the pod from the Service load balancer on the first failure for faster traffic isolation.

## Upgrading

```bash
helm upgrade trident ./helm/trident --reuse-values \
  --set goApi.image.tag=v0.2.0 \
  --set indexer.image.tag=v0.2.0 \
  --set grpcApi.image.tag=v0.2.0
```

## Uninstalling

```bash
helm uninstall trident --namespace trident
kubectl delete namespace trident
# Retain the secret if you plan to reinstall:
# kubectl -n trident delete secret trident-secrets
```
