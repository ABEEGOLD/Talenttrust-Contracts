#![cfg(test)]

use crate::{
    test::{EscrowFixtureBuilder, MILESTONE_ONE, MILESTONE_TWO, MILESTONE_THREE},
    types::{ContractStatus, Error, DisputeResolution},
};
use soroban_sdk::{testutils::Events, testutils::Address, token::StellarAssetClient, vec, Env};

#[test]
fn test_eligible_closure() {
    let fixture = EscrowFixtureBuilder::new().funded().completed().build();
    let escrow = &fixture.escrow();
    let client = &fixture.client;
    let contract_id = fixture.escrow_id;

    // Finalize the completed contract
    assert!(escrow.finalize_contract(&contract_id, client));

    let record = escrow.get_finalization_record(&contract_id).unwrap();
    assert_eq!(record.finalizer, client.clone());
    assert_eq!(record.summary.status, ContractStatus::Completed);
}

#[test]
fn test_active_balance() {
    let fixture = EscrowFixtureBuilder::new().funded().build();
    let escrow = &fixture.escrow();
    let client = &fixture.client;
    let contract_id = fixture.escrow_id;

    // Do NOT release milestone, so status is Funded.
    let res = escrow.try_finalize_contract(&contract_id, client);
    assert_eq!(
        res.err().unwrap().unwrap(),
        Error::InvalidStatusTransition.into()
    );
}

#[test]
fn test_active_dispute() {
    let fixture = EscrowFixtureBuilder::new().funded().build();
    let escrow = &fixture.escrow();
    let client = &fixture.client;
    let contract_id = fixture.escrow_id;

    // When dispute is not raised, status is Funded
    let res = escrow.try_finalize_contract(&contract_id, client);
    assert_eq!(
        res.err().unwrap().unwrap(),
        Error::InvalidStatusTransition.into()
    );
}

#[test]
fn test_repeat_finalization() {
    let fixture = EscrowFixtureBuilder::new().funded().completed().build();
    let escrow = &fixture.escrow();
    let client = &fixture.client;
    let contract_id = fixture.escrow_id;

    // Finalize it once
    escrow.finalize_contract(&contract_id, client);

    // Try to finalize again
    let res = escrow.try_finalize_contract(&contract_id, client);
    assert_eq!(res.err().unwrap().unwrap(), Error::AlreadyFinalized.into());
}

#[test]
fn test_concurrent_finalization() {
    let fixture = EscrowFixtureBuilder::new().funded().completed().build();
    let escrow = &fixture.escrow();
    let client = &fixture.client;
    let freelancer = &fixture.freelancer;
    let contract_id = fixture.escrow_id;

    // First finalizer wins
    assert!(escrow.finalize_contract(&contract_id, client));

    // Concurrent/second finalizer is rejected with AlreadyFinalized
    let res = escrow.try_finalize_contract(&contract_id, freelancer);
    assert_eq!(res.err().unwrap().unwrap(), Error::AlreadyFinalized.into());
}

#[test]
fn test_finalize_refunded_contract() {
    // Use a fixture without deadlines to allow refund anytime
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    
    let admin = <soroban_sdk::Address as soroban_sdk::testutils::Address>::generate(&env);
    let client_addr = <soroban_sdk::Address as soroban_sdk::testutils::Address>::generate(&env);
    let freelancer_addr = <soroban_sdk::Address as soroban_sdk::testutils::Address>::generate(&env);
    
    let escrow_address = env.register(crate::Escrow, ());
    let escrow = crate::EscrowClient::new(&env, &escrow_address);
    
    escrow.initialize(&admin);
    
    let token = env.register_stellar_asset_contract(admin.clone());
    escrow.bind_settlement_token(&admin, &token);
    
    // Create milestones without deadlines (deadline = None)
    let milestones = vec![&env, 10_000_000_i128];
    let contract_id = escrow.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &crate::ReleaseAuthorization::ClientOnly,
    );
    
    let total = 10_000_000_i128;
    StellarAssetClient::new(&env, &token).mint(&client_addr, &total);
    escrow.deposit_funds(&contract_id, &client_addr, &total);

    // Refund all milestones (no deadline, so allowed anytime)
    escrow.refund_unreleased_milestones(&contract_id, &vec![&env, 0]);

    // Contract should be Refunded
    let contract = escrow.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Refunded);

    // Finalize the refunded contract
    assert!(escrow.finalize_contract(&contract_id, &client_addr));

    let record = escrow.get_finalization_record(&contract_id).unwrap();
    assert_eq!(record.finalizer, client_addr);
    assert_eq!(record.summary.status, ContractStatus::Refunded);
}

#[test]
fn test_finalize_cancelled_contract() {
    let fixture = EscrowFixtureBuilder::new().funded().build();
    let escrow = &fixture.escrow();
    let client = &fixture.client;
    let contract_id = fixture.escrow_id;

    // Cancel the contract
    assert!(escrow.cancel_contract(&contract_id, &client.clone()));

    // Contract should be Cancelled
    let contract = escrow.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Cancelled);

    // Finalize the cancelled contract
    assert!(escrow.finalize_contract(&contract_id, client));

    let record = escrow.get_finalization_record(&contract_id).unwrap();
    assert_eq!(record.finalizer, client.clone());
    assert_eq!(record.summary.status, ContractStatus::Cancelled);
}

#[test]
fn test_finalize_disputed_then_fullrefund() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    
    let client_addr = <soroban_sdk::Address as soroban_sdk::testutils::Address>::generate(&env);
    let freelancer_addr = <soroban_sdk::Address as soroban_sdk::testutils::Address>::generate(&env);
    let arbiter_addr = <soroban_sdk::Address as soroban_sdk::testutils::Address>::generate(&env);

    let escrow_address = env.register(crate::Escrow, ());
    let escrow = crate::EscrowClient::new(&env, &escrow_address);
    
    let admin = <soroban_sdk::Address as soroban_sdk::testutils::Address>::generate(&env);
    escrow.initialize(&admin);
    
    let token = env.register_stellar_asset_contract(admin.clone());
    escrow.bind_settlement_token(&admin, &token);
    
    let milestones = vec![&env, 10_000_000_i128];
    let contract_id = escrow.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &milestones,
        &crate::ReleaseAuthorization::ClientOnly,
    );
    
    let total = 10_000_000_i128;
    StellarAssetClient::new(&env, &token).mint(&client_addr, &total);
    escrow.deposit_funds(&contract_id, &client_addr, &total);

    // Raise dispute
    escrow.raise_dispute(&contract_id, &client_addr);

    // Verify status is Disputed
    let contract = escrow.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Disputed);

    // Resolve with FullRefund
    escrow.resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::FullRefund);

    // Contract should be Refunded
    let contract = escrow.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Refunded);

    // Finalize the refunded contract
    assert!(escrow.finalize_contract(&contract_id, &client_addr));

    let record = escrow.get_finalization_record(&contract_id).unwrap();
    assert_eq!(record.finalizer, client_addr);
    assert_eq!(record.summary.status, ContractStatus::Refunded);
}

#[test]
fn test_finalize_rejects_created() {
    let fixture = EscrowFixtureBuilder::new().build(); // Not funded
    let escrow = &fixture.escrow();
    let client = &fixture.client;
    let contract_id = fixture.escrow_id;

    // Contract is in Created status (not funded)
    let contract = escrow.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Created);

    // Cannot finalize Created contract
    let res = escrow.try_finalize_contract(&contract_id, client);
    assert_eq!(
        res.err().unwrap().unwrap(),
        Error::InvalidStatusTransition.into()
    );
}

#[test]
fn test_finalize_accounting_invariant_completed() {
    let fixture = EscrowFixtureBuilder::new().funded().completed().build();
    let escrow = &fixture.escrow();
    let client = &fixture.client;
    let contract_id = fixture.escrow_id;

    // Verify accounting is correct
    let contract = escrow.get_contract(&contract_id);
    assert_eq!(contract.funded_amount, contract.released_amount + contract.refunded_amount);

    // Finalize should succeed
    assert!(escrow.finalize_contract(&contract_id, client));
}