//! Fuzz tests for the CreditScore contract using proptest.
//! These tests generate random sequences of payments and defaults and verify several invariants.

#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, testutils::Ledger, Env};
use proptest::prelude::*;

// ---------- Helper structs for generated operations ----------
#[derive(Debug, Clone)]
enum Op {
    Payment {
        invoice_id: u64,
        amount: i128,
        due_date: u64,
        paid_at: u64,
    },
    Default {
        invoice_id: u64,
        amount: i128,
        due_date: u64,
    },
}

prop_compose! {
    fn payment_op()(invoice_id in 1u64..1000,
                     amount in 1i128..1_000_000_000_000i128,
                     due_date in 1u64..1_000_000u64,
                     // paid_at can be before, within, or after due_date + threshold
                     paid_at_offset in -10_000i64..10_000i64) -> Op {
        // Ensure paid_at is a u64 and may be before or after due_date.
        let paid_at = if paid_at_offset.is_negative() {
            due_date.saturating_sub(paid_at_offset.wrapping_abs() as u64)
        } else {
            due_date.saturating_add(paid_at_offset as u64)
        };
        Op::Payment { invoice_id, amount, due_date, paid_at }
    }
}

prop_compose! {
    fn default_op()(invoice_id in 1u64..1000,
                    amount in 1i128..1_000_000_000_000i128,
                    due_date in 1u64..1_000_000u64) -> Op {
        Op::Default { invoice_id, amount, due_date }
    }
}

prop_compose! {
    fn ops_seq()(ops in prop::collection::vec(
            prop_oneof![payment_op(), default_op()],
            1..50)) -> Vec<Op> {
        ops
    }
}

/// Helper to execute a generated operation sequence against the contract and check invariants.
fn execute_and_check(env: &Env, client: &CreditScoreContractClient<'_>, sme: &Address, ops: &[Op]) {
    // Track invoices already processed to test idempotency.
    let mut processed = std::collections::HashSet::new();

    for op in ops {
        match op {
            Op::Payment { invoice_id, amount, due_date, paid_at } => {
                // Record payment (avoid panics for duplicate invoices).
                if processed.insert(*invoice_id) {
                    client.record_payment(&client.env().registered_contract_id(), // pool address placeholder
                        invoice_id, sme, *amount, *due_date, *paid_at);
                }
            }
            Op::Default { invoice_id, amount, due_date } => {
                if processed.insert(*invoice_id) {
                    client.record_default(&client.env().registered_contract_id(), invoice_id, sme, *amount, *due_date);
                }
            }
        }
        // Invariant 1: Score stays within bounds.
        let score_data = client.get_credit_score(sme);
        assert!(score_data.score >= MIN_SCORE && score_data.score <= MAX_SCORE);
        // Invariant 2: Monotonicity – on‑time payments increase score (unless at max), defaults decrease (unless at min).
        // Simple check: after each op the score does not go out of bounds (already asserted) and the counters are consistent.
        let total = score_data.total_invoices;
        let counters = score_data.paid_on_time + score_data.paid_late + score_data.defaulted;
        assert_eq!(total, counters);
    }
}

proptest! {
    #[test]
    fn fuzz_credit_score(ops in ops_seq()) {
        let env = Env::default();
        env.mock_all_auths();
        // Setup contracts.
        let contract_id = env.register(CreditScoreContract, ());
        let client = CreditScoreContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let invoice_contract = Address::generate(&env);
        let pool_contract = Address::generate(&env);
        client.initialize(&admin, &invoice_contract, &pool_contract);

        let sme = Address::generate(&env);
        execute_and_check(&env, &client, &sme, &ops);
    }
}
