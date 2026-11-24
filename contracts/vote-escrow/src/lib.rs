#![no_std]

mod storage;
mod types;

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

pub use types::{LockDurationBreakdown, LockRecord, LockStatus, UnlockPressure};

const LONG_LOCK_THRESHOLD: u64 = 52_560_000;

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Lock(u64),
    TotalLockedAmount,
    TotalLocks,
    ShortLocks,
    LongLocks,
    UnlockedAmount,
}

#[contract]
pub struct VoteEscrow;

#[contractimpl]
impl VoteEscrow {
    pub fn init(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    pub fn lock(env: Env, locker: Address, lock_id: u64, amount: i128, unlock_time: u64) {
        locker.require_auth();
        assert!(amount > 0, "Amount must be positive");
        assert!(
            unlock_time > env.ledger().timestamp(),
            "Unlock time must be in future"
        );
        assert!(
            storage::get_lock(&env, lock_id).is_none(),
            "Lock ID already exists"
        );

        let locked_at = env.ledger().timestamp();
        let duration = unlock_time - locked_at;

        let is_long = duration > LONG_LOCK_THRESHOLD;

        let record = LockRecord {
            lock_id,
            locker: locker.clone(),
            amount,
            locked_at,
            unlock_time,
            unlocked: false,
        };

        storage::set_lock(&env, &record);
        storage::add_total_locked_amount(&env, amount);
        storage::increment_total_locks(&env);

        if is_long {
            storage::increment_long_locks(&env);
        } else {
            storage::increment_short_locks(&env);
        }
    }

    pub fn unlock(env: Env, admin: Address, lock_id: u64) -> i128 {
        admin.require_auth();

        let mut record = storage::get_lock(&env, lock_id).expect("Lock not found");
        assert!(!record.unlocked, "Already unlocked");
        assert!(
            env.ledger().timestamp() >= record.unlock_time,
            "Lock period not expired"
        );
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        assert!(
            admin == stored_admin || admin == record.locker,
            "Unauthorized caller"
        );

        record.unlocked = true;
        storage::set_lock(&env, &record);
        storage::add_unlocked_amount(&env, record.amount);

        record.amount
    }

    pub fn lock_duration_breakdown(env: Env) -> LockDurationBreakdown {
        let configured = env.storage().instance().has(&DataKey::Admin);

        LockDurationBreakdown {
            configured,
            total_locked_amount: storage::get_total_locked_amount(&env),
            total_locks: storage::get_total_locks(&env),
            short_locks: storage::get_short_locks(&env),
            long_locks: storage::get_long_locks(&env),
            unlocked_amount: storage::get_unlocked_amount(&env),
        }
    }

    pub fn unlock_pressure(env: Env, lock_id: u64) -> UnlockPressure {
        let now = env.ledger().timestamp();
        let configured = env.storage().instance().has(&DataKey::Admin);

        let Some(record) = storage::get_lock(&env, lock_id) else {
            return UnlockPressure {
                lock_id,
                configured,
                exists: false,
                status: if configured {
                    LockStatus::Locked
                } else {
                    LockStatus::NotConfigured
                },
                amount: 0,
                locked_at: 0,
                unlock_time: 0,
                now,
                time_remaining: 0,
            };
        };

        let status = if record.unlocked {
            LockStatus::Unlocked
        } else if now >= record.unlock_time {
            LockStatus::Unlockable
        } else {
            LockStatus::Locked
        };

        let time_remaining = if now >= record.unlock_time {
            0
        } else {
            let diff = record.unlock_time - now;
            i64::try_from(diff).unwrap_or(i64::MAX)
        };

        UnlockPressure {
            lock_id,
            configured,
            exists: true,
            status,
            amount: record.amount,
            locked_at: record.locked_at,
            unlock_time: record.unlock_time,
            now,
            time_remaining,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    fn setup(env: &Env) -> (Address, VoteEscrowClient<'_>) {
        env.mock_all_auths();
        let contract_id = env.register(VoteEscrow, ());
        let client = VoteEscrowClient::new(env, &contract_id);
        let admin = Address::generate(env);
        client.init(&admin);
        (admin, client)
    }

    #[test]
    fn test_init() {
        let env = Env::default();
        let _ = setup(&env);
    }

    #[test]
    fn test_lock_and_unlock() {
        let env = Env::default();
        env.ledger().set_timestamp(1000);

        let (admin, client) = setup(&env);
        let locker = Address::generate(&env);

        client.lock(&locker, &1, &1000, &3000);

        let breakdown = client.lock_duration_breakdown();
        assert_eq!(breakdown.total_locked_amount, 1000);
        assert_eq!(breakdown.total_locks, 1);

        let mut ledger = env.ledger().get();
        ledger.timestamp = 3100;
        env.ledger().set(ledger);

        let unlocked_amount = client.unlock(&admin, &1);
        assert_eq!(unlocked_amount, 1000);

        let breakdown = client.lock_duration_breakdown();
        assert_eq!(breakdown.unlocked_amount, 1000);
    }

    #[test]
    fn test_unlock_pressure_missing() {
        let env = Env::default();
        let (_admin, client) = setup(&env);

        let pressure = client.unlock_pressure(&999);
        assert!(!pressure.exists);
        assert!(pressure.configured);
    }

    #[test]
    #[should_panic(expected = "Lock ID already exists")]
    fn test_duplicate_lock_id_rejected() {
        let env = Env::default();
        env.ledger().set_timestamp(1000);
        let (_admin, client) = setup(&env);
        let locker = Address::generate(&env);

        client.lock(&locker, &1, &500, &2000);
        assert_eq!(client.lock_duration_breakdown().total_locks, 1);
        assert_eq!(client.lock_duration_breakdown().total_locked_amount, 500);

        // Duplicate lock ID panics and doesn't overwrite
        client.lock(&locker, &1, &300, &2500);
    }

    #[test]
    #[should_panic(expected = "Unauthorized caller")]
    fn test_unauthorized_unlock_rejected() {
        let env = Env::default();
        env.ledger().set_timestamp(1000);
        let (_admin, client) = setup(&env);
        let locker = Address::generate(&env);
        let imposter = Address::generate(&env);

        client.lock(&locker, &1, &500, &2000);

        let mut ledger = env.ledger().get();
        ledger.timestamp = 2500;
        env.ledger().set(ledger);

        client.unlock(&imposter, &1);
    }

    #[test]
    fn test_locker_can_unlock() {
        let env = Env::default();
        env.ledger().set_timestamp(1000);
        let (_admin, client) = setup(&env);
        let locker = Address::generate(&env);

        client.lock(&locker, &1, &500, &2000);

        let mut ledger = env.ledger().get();
        ledger.timestamp = 2500;
        env.ledger().set(ledger);

        let amt = client.unlock(&locker, &1);
        assert_eq!(amt, 500);
    }

    #[test]
    fn test_unlock_pressure_distant_unlock_does_not_wrap() {
        let env = Env::default();
        env.ledger().set_timestamp(1000);
        let (_admin, client) = setup(&env);
        let locker = Address::generate(&env);

        // Distant future timestamp: unlock_time - now > i64::MAX
        let distant_future = 1000u64 + (i64::MAX as u64) + 1_000_000u64;
        client.lock(&locker, &99, &500, &distant_future);

        let pressure = client.unlock_pressure(&99);
        assert!(pressure.exists);
        assert_eq!(pressure.status, LockStatus::Locked);
        assert!(pressure.time_remaining > 0);
        assert_eq!(pressure.time_remaining, i64::MAX);
    }
}
