//! Reference SEP-41 fungible token (issue #258).
//!
//! A minimal, deliberately small token implementation whose events match the
//! shapes `crates/indexer/src/parser/token_events.rs` decodes: `transfer`,
//! `mint`, `burn`, `clawback` publish `(Address, Address)`-ish topics with an
//! `i128` amount body; `approve` publishes an `(i128, u32)` body. It also
//! exposes `name()`/`symbol()`/`decimals()` for the token metadata resolver
//! (issue #263) to simulate against. Full SEP-41 depth (fees, multi-admin,
//! etc.) is out of scope here — see issue #259.
//!
//! Events are published via `env.events().publish(...)` (deprecated in
//! favour of `#[contractevent]` in this SDK version) deliberately, not
//! migrated: `#[contractevent]` serialises to a different, macro-defined
//! shape, and this contract exists specifically to emit the bare
//! `(topics, i128)` wire format `token_events.rs` decodes.
#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, String,
};

#[derive(Clone)]
#[contracttype]
enum DataKey {
    Admin,
    Name,
    Symbol,
    Decimals,
    Balance(Address),
    Allowance(Address, Address),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TokenError {
    AlreadyInitialized = 1,
    InsufficientBalance = 2,
    InsufficientAllowance = 3,
}

#[contract]
pub struct Token;

#[contractimpl]
impl Token {
    /// One-time setup. Must be called before any other function.
    pub fn initialize(
        env: Env,
        admin: Address,
        decimals: u32,
        name: String,
        symbol: String,
    ) -> Result<(), TokenError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(TokenError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Decimals, &decimals);
        env.storage().instance().set(&DataKey::Name, &name);
        env.storage().instance().set(&DataKey::Symbol, &symbol);
        Ok(())
    }

    pub fn name(env: Env) -> String {
        env.storage().instance().get(&DataKey::Name).unwrap()
    }

    pub fn symbol(env: Env) -> String {
        env.storage().instance().get(&DataKey::Symbol).unwrap()
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::Decimals).unwrap()
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(id))
            .unwrap_or(0)
    }

    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        env.storage()
            .temporary()
            .get(&DataKey::Allowance(from, spender))
            .unwrap_or(0)
    }

    /// Mint `amount` to `to`. Only the admin may call this.
    pub fn mint(env: Env, to: Address, amount: i128) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        Self::adjust_balance(&env, &to, amount);

        env.events()
            .publish((symbol_short!("mint"), admin, to), amount);
    }

    /// Move `amount` from the caller (`from`) to `to`.
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        Self::do_transfer(&env, &from, &to, amount);

        env.events()
            .publish((symbol_short!("transfer"), from, to), amount);
    }

    /// Move `amount` from `from` to `to` using a prior `approve` allowance.
    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        Self::spend_allowance(&env, &from, &spender, amount);
        Self::do_transfer(&env, &from, &to, amount);

        env.events()
            .publish((symbol_short!("transfer"), from, to), amount);
    }

    /// Authorise `spender` to move up to `amount` from the caller (`from`),
    /// expiring at `expiration_ledger`.
    pub fn approve(
        env: Env,
        from: Address,
        spender: Address,
        amount: i128,
        expiration_ledger: u32,
    ) {
        from.require_auth();
        env.storage()
            .temporary()
            .set(&DataKey::Allowance(from.clone(), spender.clone()), &amount);
        if amount > 0 {
            env.storage().temporary().extend_ttl(
                &DataKey::Allowance(from.clone(), spender.clone()),
                expiration_ledger.saturating_sub(env.ledger().sequence()),
                expiration_ledger.saturating_sub(env.ledger().sequence()),
            );
        }

        env.events().publish(
            (symbol_short!("approve"), from, spender),
            (amount, expiration_ledger),
        );
    }

    /// Burn `amount` from the caller (`from`).
    pub fn burn(env: Env, from: Address, amount: i128) {
        from.require_auth();
        Self::adjust_balance(&env, &from, -amount);

        env.events().publish((symbol_short!("burn"), from), amount);
    }

    /// Admin-forced burn from `from`, bypassing `from`'s authorisation.
    pub fn clawback(env: Env, from: Address, amount: i128) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        Self::adjust_balance(&env, &from, -amount);

        env.events()
            .publish((symbol_short!("clawback"), admin, from), amount);
    }

    fn do_transfer(env: &Env, from: &Address, to: &Address, amount: i128) {
        Self::adjust_balance(env, from, -amount);
        Self::adjust_balance(env, to, amount);
    }

    fn adjust_balance(env: &Env, id: &Address, delta: i128) {
        let key = DataKey::Balance(id.clone());
        let balance: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        let new_balance = balance + delta;
        if new_balance < 0 {
            panic!("insufficient balance");
        }
        env.storage().persistent().set(&key, &new_balance);
    }

    fn spend_allowance(env: &Env, from: &Address, spender: &Address, amount: i128) {
        let key = DataKey::Allowance(from.clone(), spender.clone());
        let allowance: i128 = env.storage().temporary().get(&key).unwrap_or(0);
        if allowance < amount {
            panic!("insufficient allowance");
        }
        env.storage().temporary().set(&key, &(allowance - amount));
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn setup(env: &Env) -> (Address, TokenClient<'_>) {
        let admin = Address::generate(env);
        let contract_id = env.register(Token, ());
        let client = TokenClient::new(env, &contract_id);
        client.initialize(
            &admin,
            &7,
            &String::from_str(env, "Example Token"),
            &String::from_str(env, "EXT"),
        );
        (admin, client)
    }

    #[test]
    fn mint_and_transfer_move_balances() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, client) = setup(&env);

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        client.mint(&alice, &1_000);
        assert_eq!(client.balance(&alice), 1_000);

        client.transfer(&alice, &bob, &400);
        assert_eq!(client.balance(&alice), 600);
        assert_eq!(client.balance(&bob), 400);
    }

    #[test]
    fn approve_then_transfer_from_respects_allowance() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, client) = setup(&env);

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let carol = Address::generate(&env);

        client.mint(&alice, &1_000);
        client.approve(&alice, &bob, &300, &(env.ledger().sequence() + 100));
        assert_eq!(client.allowance(&alice, &bob), 300);

        client.transfer_from(&bob, &alice, &carol, &200);
        assert_eq!(client.balance(&alice), 800);
        assert_eq!(client.balance(&carol), 200);
        assert_eq!(client.allowance(&alice, &bob), 100);
    }

    #[test]
    fn burn_reduces_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, client) = setup(&env);

        let alice = Address::generate(&env);
        client.mint(&alice, &500);
        client.burn(&alice, &200);
        assert_eq!(client.balance(&alice), 300);
    }

    #[test]
    fn metadata_matches_initialize_args() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, client) = setup(&env);

        assert_eq!(client.decimals(), 7);
        assert_eq!(client.name(), String::from_str(&env, "Example Token"));
        assert_eq!(client.symbol(), String::from_str(&env, "EXT"));
    }
}
