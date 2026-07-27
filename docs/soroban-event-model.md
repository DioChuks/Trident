# Soroban Event Model

This document describes how Trident interprets and decodes Soroban contract events emitted on the Stellar network.

## Event Structure

Every Soroban event exposed by Trident has the following fields:

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Monotonically increasing cursor (ledger-sequence + event-index) |
| `ledger` | number | Ledger sequence in which the event was emitted |
| `timestamp` | string (ISO 8601) | Close time of the containing ledger |
| `contract_id` | string | StrKey-encoded contract address (`C…`) |
| `topics` | array | Decoded XDR topic values |
| `data` | any | Decoded XDR data payload |
| `type` | string | `contract`, `system`, or `diagnostic` |

### topics vs data

Soroban events carry two distinct payloads:

- **topics** — an ordered list of up to four `ScVal` values used for filtering. The first topic is conventionally the event name (e.g. `"transfer"`, `"mint"`).
- **data** — a single `ScVal` value that holds the event body (amounts, addresses, metadata).

## XDR-to-JSON Decoding

Trident decodes all `ScVal` types to their natural JSON equivalents:

| ScVal type | JSON representation |
|------------|---------------------|
| `ScvSymbol` | string |
| `ScvString` | string |
| `ScvBool` | boolean |
| `ScvAddress` | string (StrKey) |
| `ScvMap` | object |
| `ScvVec` | array |
| `ScvI128` / `ScvU128` / `ScvI256` / `ScvU256` | **string** (see below) |
| `ScvI64` / `ScvU64` / `ScvI32` / `ScvU32` | number |
| `ScvBytes` | string (base64) |
| `ScvVoid` | `null` |

### Big-integer encoding

128-bit and 256-bit integer types are serialised as **decimal strings** to avoid precision loss in JavaScript (`Number.MAX_SAFE_INTEGER` is only 53 bits). Consumers must parse these with a BigInt library before doing arithmetic.

```json
{
  "topics": ["transfer"],
  "data": {
    "from": "GABC…",
    "to":   "GDEF…",
    "amount": "1000000000000"
  }
}
```

## Ordering and Idempotency Guarantees

- Events are delivered in **ledger order** within a stream. Within a single ledger, events follow the transaction-index then event-index order defined by the Stellar protocol.
- Each event has a stable `id` (cursor). Re-subscribing with `cursor=<last-id>` resumes the stream without gaps or duplicates.
- The indexer writes events exactly once per ledger; retries are idempotent (duplicate ledgers are skipped).

## Related

- [Stream events API](./stream-events.md)
- [Indexer event filtering](./indexer-event-filtering.md)
- [Specification](./SPECIFICATION.md)
