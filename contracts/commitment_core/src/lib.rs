#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype,
    Address, Env, String, Symbol, Vec,
    Val, IntoVal,
};
use soroban_sdk::token::Client as TokenClient;

/* -------------------- STORAGE KEYS -------------------- */

const ADMIN_KEY: Symbol = Symbol::short("ADMIN");
const NFT_KEY: Symbol = Symbol::short("NFT");
const COMMITMENTS_KEY: Symbol = Symbol::short("COMMS");

/* -------------------- DATA TYPES -------------------- */

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitmentRules {
    pub duration_days: u32,
    pub max_loss_percent: u32,
    pub commitment_type: String,
    pub early_exit_penalty: u32,
    pub min_fee_threshold: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commitment {
    pub commitment_id: String,
    pub owner: Address,
    pub nft_token_id: u32,
    pub rules: CommitmentRules,
    pub amount: i128,
    pub asset_address: Address,
    pub created_at: u64,
    pub expires_at: u64,
    pub current_value: i128,
    pub status: String, // active | settled | violated | early_exit
}

/* -------------------- CONTRACT -------------------- */

#[contract]
pub struct CommitmentCoreContract;

#[contractimpl]
impl CommitmentCoreContract {

    /* ---------- INITIALIZE ---------- */

    pub fn initialize(e: Env, admin: Address, nft_contract: Address) {
        admin.require_auth();

        e.storage().instance().set(&ADMIN_KEY, &admin);
        e.storage().instance().set(&NFT_KEY, &nft_contract);

        let empty: Vec<Commitment> = Vec::new(&e);
        e.storage().instance().set(&COMMITMENTS_KEY, &empty);
    }

    /* ---------- CREATE COMMITMENT ---------- */

    pub fn create_commitment(
        e: Env,
        owner: Address,
        amount: i128,
        asset_address: Address,
        rules: CommitmentRules,
    ) -> String {
        owner.require_auth();

        if amount <= 0 {
            panic!("Invalid amount");
        }

        let now = e.ledger().timestamp();
        let expires_at = now + (rules.duration_days as u64 * 86400);

        let commitment_id =
            String::from_str(&e, "commitment");

        // Transfer asset into contract
        TokenClient::new(&e, &asset_address)
            .transfer(&owner, &e.current_contract_address(), &amount);

        // Mint NFT
        let nft_contract: Address =
            e.storage().instance().get(&NFT_KEY).unwrap();

        let mut mint_args = Vec::<Val>::new(&e);
        mint_args.push_back(owner.clone().into_val(&e));
        mint_args.push_back(commitment_id.clone().into_val(&e));

        let nft_token_id: u32 = e.invoke_contract(
            &nft_contract,
            &Symbol::short("mint"),
            mint_args,
        );

        let mut commitments: Vec<Commitment> =
            e.storage().instance().get(&COMMITMENTS_KEY).unwrap();

        commitments.push_back(Commitment {
            commitment_id: commitment_id.clone(),
            owner: owner.clone(),
            nft_token_id,
            rules,
            amount,
            asset_address,
            created_at: now,
            expires_at,
            current_value: amount,
            status: String::from_str(&e, "active"),
        });

        e.storage().instance().set(&COMMITMENTS_KEY, &commitments);

        e.events().publish(
            (Symbol::short("CommitmentCreated"),),
            (commitment_id.clone(), owner, amount, now),
        );

        commitment_id
    }

    /* ---------- GET COMMITMENT ---------- */

    pub fn get_commitment(e: Env, commitment_id: String) -> Commitment {
        let commitments: Vec<Commitment> =
            e.storage().instance().get(&COMMITMENTS_KEY).unwrap();

        for c in commitments.iter() {
            if c.commitment_id == commitment_id {
                return c;
            }
        }

        panic!("Commitment not found");
    }

    /* ---------- UPDATE VALUE ---------- */

    pub fn update_value(e: Env, commitment_id: String, new_value: i128) {
        let admin: Address =
            e.storage().instance().get(&ADMIN_KEY).unwrap();
        admin.require_auth();

        let mut commitments: Vec<Commitment> =
            e.storage().instance().get(&COMMITMENTS_KEY).unwrap();

        for mut c in commitments.iter() {
            if c.commitment_id == commitment_id {
                if c.status != String::from_str(&e, "active") {
                    panic!("Not active");
                }

                c.current_value = new_value;

                let loss_percent =
                    (c.amount - new_value) * 100 / c.amount;

                if loss_percent > c.rules.max_loss_percent as i128 {
                    c.status = String::from_str(&e, "violated");
                }

                e.events().publish(
                    (Symbol::short("ValueUpdated"),),
                    (commitment_id, new_value),
                );

                e.storage().instance().set(&COMMITMENTS_KEY, &commitments);
                return;
            }
        }

        panic!("Commitment not found");
    }

    /* ---------- CHECK VIOLATIONS ---------- */

    pub fn check_violations(e: Env, commitment_id: String) -> bool {
        let c = Self::get_commitment(e.clone(), commitment_id);
        let now = e.ledger().timestamp();

        if now >= c.expires_at {
            return true;
        }

        let loss_percent =
            (c.amount - c.current_value) * 100 / c.amount;

        loss_percent > c.rules.max_loss_percent as i128
    }

    /* ---------- SETTLEMENT ---------- */

    pub fn settle(e: Env, commitment_id: String) {
        let now = e.ledger().timestamp();

        let mut commitments: Vec<Commitment> =
            e.storage().instance().get(&COMMITMENTS_KEY).unwrap();

        for mut c in commitments.iter() {
            if c.commitment_id == commitment_id {
                if c.status != String::from_str(&e, "active") {
                    panic!("Already settled");
                }

                if now < c.expires_at {
                    panic!("Commitment not expired");
                }

                // Transfer settlement amount
                TokenClient::new(&e, &c.asset_address)
                    .transfer(
                        &e.current_contract_address(),
                        &c.owner,
                        &c.current_value,
                    );

                c.status = String::from_str(&e, "settled");

                // Mark NFT settled
                let nft_contract: Address =
                    e.storage().instance().get(&NFT_KEY).unwrap();

                let mut args = Vec::<Val>::new(&e);
                args.push_back(c.nft_token_id.into_val(&e));

                e.invoke_contract::<()>(
                    &nft_contract,
                    &Symbol::short("Mark_settled"),
                    args,
                );

                e.events().publish(
                    (Symbol::short("Commitment Settled"),),
                    (
                        commitment_id,
                        c.owner.clone(),
                        c.current_value,
                        now,
                    ),
                );

                e.storage().instance().set(&COMMITMENTS_KEY, &commitments);
                return;
            }
        }

        panic!("Commitment not found");
    }

    /* ---------- EARLY EXIT ---------- */

    pub fn early_exit(e: Env, commitment_id: String, caller: Address) {
        caller.require_auth();

        let mut commitments: Vec<Commitment> =
            e.storage().instance().get(&COMMITMENTS_KEY).unwrap();

        for mut c in commitments.iter() {
            if c.commitment_id == commitment_id {
                if caller != c.owner {
                    panic!("Unauthorized");
                }

                if c.status != String::from_str(&e, "active") {
                    panic!("Not active");
                }

                let penalty =
                    c.current_value * c.rules.early_exit_penalty as i128 / 100;
                let payout = c.current_value - penalty;

                TokenClient::new(&e, &c.asset_address)
                    .transfer(
                        &e.current_contract_address(),
                        &c.owner,
                        &payout,
                    );

                c.status = String::from_str(&e, "early_exit");

                e.events().publish(
                    (Symbol::short("EarlyExit"),),
                    (commitment_id, payout),
                );

                e.storage().instance().set(&COMMITMENTS_KEY, &commitments);
                return;
            }
        }

        panic!("Commitment not found");
    }

    /* ---------- ALLOCATE ---------- */

    pub fn allocate(
        e: Env,
        commitment_id: String,
        target_pool: Address,
        amount: i128,
    ) {
        let admin: Address =
            e.storage().instance().get(&ADMIN_KEY).unwrap();
        admin.require_auth();

        let commitments: Vec<Commitment> =
            e.storage().instance().get(&COMMITMENTS_KEY).unwrap();

        for c in commitments.iter() {
            if c.commitment_id == commitment_id {
                if c.status != String::from_str(&e, "active") {
                    panic!("Not active");
                }

                TokenClient::new(&e, &c.asset_address)
                    .transfer(
                        &e.current_contract_address(),
                        &target_pool,
                        &amount,
                    );

                e.events().publish(
                    (Symbol::short("Allocated"),),
                    (commitment_id, target_pool, amount),
                );

                return;
            }
        }

        panic!("Commitment not found");
    }
}
