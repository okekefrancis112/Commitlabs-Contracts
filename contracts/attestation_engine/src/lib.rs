#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Symbol, Address, Env, String, Vec, Map,
    IntoVal, TryIntoVal, Val,
};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attestation {
    pub commitment_id: u32,  // Changed from String to u32
    pub timestamp: u64,
    pub attestation_type: String, // "health_check", "violation", "fee_generation", "drawdown"
    pub data: Map<String, String>, // Flexible data structure
    pub is_compliant: bool,
    pub verified_by: Address,
}

// Import Commitment types from commitment_core (define locally for cross-contract calls)
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
    pub commitment_id: u32,  // Changed from String to u32
    pub owner: Address,
    pub nft_token_id: u32,
    pub rules: CommitmentRules,
    pub amount: i128,
    pub asset_address: Address,
    pub created_at: u64,
    pub expires_at: u64,
    pub current_value: i128,
    pub status: String,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthMetrics {
    pub commitment_id: u32,  // Changed from String to u32
    pub current_value: i128,
    pub initial_value: i128,
    pub drawdown_percent: i128,
    pub fees_generated: i128,
    pub volatility_exposure: i128,
    pub last_attestation: u64,
    pub compliance_score: u32, // 0-100
}

#[contract]
pub struct AttestationEngineContract;

#[contractimpl]
impl AttestationEngineContract {
    /// Initialize the attestation engine
    pub fn initialize(e: Env, admin: Address, commitment_core: Address) {
        e.storage().instance().set(&symbol_short!("ADMIN"), &admin);
        e.storage().instance().set(&symbol_short!("CORE"), &commitment_core);
    }

    // ========================================================================
    // Access Control
    // ========================================================================

    /// Add an authorized recorder (only admin can call)
    pub fn add_authorized_recorder(e: Env, caller: Address, recorder: Address) {
        caller.require_auth();
        
        // Verify caller is admin
        let admin: Address = e.storage()
            .instance()
            .get(&symbol_short!("ADMIN"))
            .unwrap_or_else(|| panic!("Contract not initialized"));
        
        if caller != admin {
            panic!("Unauthorized: only admin can add recorders");
        }
        
        // Add recorder to authorized list
        let key = (symbol_short!("AUTHREC"), recorder.clone());
        e.storage().instance().set(&key, &true);
        
        // Emit event
        e.events().publish(
            (Symbol::new(&e, "RecorderAdded"),),
            (recorder,)
        );
    }

    /// Check if an address is authorized to record events
    fn is_authorized_recorder(e: &Env, recorder: &Address) -> bool {
        // Admin is always authorized
        if let Some(admin) = e.storage()
            .instance()
            .get::<Symbol, Address>(&symbol_short!("ADMIN")) {
            if *recorder == admin {
                return true;
            }
        }
        
        // Check if recorder is in authorized list
        let key = (symbol_short!("AUTHREC"), recorder.clone());
        e.storage().instance().get(&key).unwrap_or(false)
    }

    // ========================================================================
    // Health Metrics Storage Helpers
    // ========================================================================

    /// Load health metrics from storage or create new ones
    fn load_or_create_health_metrics(e: &Env, commitment_id: u32) -> HealthMetrics {
        let key = (symbol_short!("HEALTH"), commitment_id);
        
        if let Some(metrics) = e.storage().persistent().get(&key) {
            metrics
        } else {
            // Create new metrics initialized to zero
            HealthMetrics {
                commitment_id,
                current_value: 0,
                initial_value: 0,
                drawdown_percent: 0,
                fees_generated: 0,
                volatility_exposure: 0,
                last_attestation: 0,
                compliance_score: 100, // Start with perfect score
            }
        }
    }

    /// Store health metrics
    fn store_health_metrics(e: &Env, metrics: &HealthMetrics) {
        let key = (symbol_short!("HEALTH"), metrics.commitment_id);
        e.storage().persistent().set(&key, metrics);
    }

    /// Record an attestation for a commitment
    pub fn attest(
        e: Env,
        commitment_id: u32,
        attestation_type: String,
        data: Map<String, String>,
        verified_by: Address,
    ) {
        let attestation = Attestation {
            commitment_id,
            attestation_type: attestation_type.clone(),
            data,
            timestamp: e.ledger().timestamp(),
            verified_by: verified_by.clone(),
            is_compliant: true,
        };
        
        let key = (symbol_short!("ATTS"), commitment_id);
        let mut attestations: Vec<Attestation> = e.storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&e));
            
        attestations.push_back(attestation);
        e.storage().persistent().set(&key, &attestations);
        
        e.events().publish(
            (symbol_short!("Attest"), commitment_id, verified_by.clone()),
            (attestation_type, true, e.ledger().timestamp())
        );
    }

    /// Get all attestations for a commitment
    pub fn get_attestations(e: Env, commitment_id: u32) -> Vec<Attestation> {
        let key = (symbol_short!("ATTS"), commitment_id);
        e.storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&e))
    }

    /// Get current health metrics for a commitment
    pub fn get_health_metrics(e: Env, commitment_id: u32) -> HealthMetrics {
        let commitment_core: Address = e.storage()
            .instance()
            .get(&symbol_short!("CORE"))
            .unwrap();

        let mut args = Vec::new(&e);
        args.push_back(commitment_id.into_val(&e));
        let commitment_val: Val = e.invoke_contract(
            &commitment_core,
            &Symbol::new(&e, "get_commitment"),
            args,
        );

        let commitment: Commitment = commitment_val.try_into_val(&e).unwrap();
        let attestations = Self::get_attestations(e.clone(), commitment_id);

        let initial_value = commitment.amount;
        let current_value = commitment.current_value;

        let drawdown_percent = if initial_value > 0 {
            let diff = initial_value.checked_sub(current_value).unwrap_or(0);
            diff.checked_mul(100).unwrap_or(0)
                .checked_div(initial_value).unwrap_or(0)
        } else {
            0
        };

        let fees_key = (symbol_short!("FEES"), commitment_id);
        let fees_generated: i128 = e.storage()
            .persistent()
            .get(&fees_key)
            .unwrap_or(0);

        let volatility_exposure: i128 = 0;

        let last_attestation = attestations.iter()
            .map(|att| att.timestamp)
            .max()
            .unwrap_or(0);

        let compliance_score = Self::calculate_compliance_score(e.clone(), commitment_id);

        HealthMetrics {
            commitment_id,
            current_value,
            initial_value,
            drawdown_percent,
            fees_generated,
            volatility_exposure,
            last_attestation,
            compliance_score,
        }
    }

    /// Verify commitment compliance
    pub fn verify_compliance(e: Env, commitment_id: u32) -> bool {
        let metrics = Self::get_health_metrics(e.clone(), commitment_id);
        let attestations = Self::get_attestations(e.clone(), commitment_id);

        for att in attestations.iter() {
            if !att.is_compliant {
                return false;
            }
        }

        if metrics.drawdown_percent > 100 {
            return false;
        }

        true
    }

    /// Record fee generation
    ///
    /// # Arguments
    /// * `caller` - The address calling this function (must be authorized)
    /// * `commitment_id` - The commitment ID to record fees for
    /// * `fee_amount` - The amount of fees generated
    pub fn record_fees(e: Env, caller: Address, commitment_id: u32, fee_amount: i128) {
        // 1. Verify caller authorization
        caller.require_auth();
        if !Self::is_authorized_recorder(&e, &caller) {
            panic!("Unauthorized: caller is not an authorized recorder");
        }
        
        if fee_amount <= 0 {
            panic!("fee_amount must be positive");
        }
        
        // 2. Update fees in persistent storage
        let fees_key = (symbol_short!("FEES"), commitment_id);
        let current_fees: i128 = e.storage()
            .persistent()
            .get(&fees_key)
            .unwrap_or(0);
        let new_total = current_fees.checked_add(fee_amount)
            .unwrap_or_else(|| panic!("Fee amount overflow"));
        e.storage().persistent().set(&fees_key, &new_total);
        
        // 3. Create fee attestation
        let mut data = Map::new(&e);
        data.set(String::from_str(&e, "fee_amount"), String::from_str(&e, "recorded"));
        Self::attest(e.clone(), commitment_id, String::from_str(&e, "fee_generation"), data, caller.clone());
        
        // 4. Load or create health metrics and update
        let mut metrics = Self::load_or_create_health_metrics(&e, commitment_id);
        metrics.fees_generated = new_total;
        metrics.compliance_score = Self::calculate_compliance_score(e.clone(), commitment_id);
        metrics.last_attestation = e.ledger().timestamp();
        Self::store_health_metrics(&e, &metrics);
        
        // 5. Emit FeeRecorded event
        e.events().publish(
            (symbol_short!("FeeRec"), commitment_id),
            (fee_amount, e.ledger().timestamp())
        );
    }

    /// Record drawdown event
    /// 
    /// # Arguments
    /// * `caller` - The address calling this function (must be authorized)
    /// * `commitment_id` - The commitment ID to record drawdown for
    /// * `current_value` - The current value of the commitment
    pub fn record_drawdown(e: Env, caller: Address, commitment_id: u32, current_value: i128) {
        // 1. Verify caller authorization
        caller.require_auth();
        if !Self::is_authorized_recorder(&e, &caller) {
            panic!("Unauthorized: caller is not an authorized recorder");
        }
        
        // 2. Get commitment from core contract to retrieve initial amount and max_loss_percent
        let commitment_core: Address = e.storage()
            .instance()
            .get(&symbol_short!("CORE"))
            .unwrap_or_else(|| panic!("Core contract not set"));
        
        let mut args = Vec::new(&e);
        args.push_back(commitment_id.into_val(&e));
        let commitment_val: Val = e.invoke_contract(
            &commitment_core,
            &Symbol::new(&e, "get_commitment"),
            args,
        );
        let commitment: Commitment = commitment_val.try_into_val(&e)
            .unwrap_or_else(|_| panic!("Failed to get commitment"));
        
        // 3. Calculate drawdown percentage: ((initial - current) / initial) * 100
        let initial_value = commitment.amount;
        let drawdown_percent = if initial_value > 0 {
            let diff = initial_value.checked_sub(current_value).unwrap_or(0);
            diff.checked_mul(100).unwrap_or(0)
                .checked_div(initial_value).unwrap_or(0)
        } else {
            0
        };
        
        // 4. Load or create health metrics
        let mut metrics = Self::load_or_create_health_metrics(&e, commitment_id);
        
        // 5. Update health metrics
        metrics.current_value = current_value;
        metrics.initial_value = initial_value;
        metrics.drawdown_percent = drawdown_percent;
        
        // 6. Check for violation
        let max_loss_percent = commitment.rules.max_loss_percent as i128;
        let is_violation = drawdown_percent > max_loss_percent;
        
        if is_violation {
            // Create violation attestation
            let violation_data = Map::new(&e);
            let violation_attestation = Attestation {
                commitment_id,
                attestation_type: String::from_str(&e, "violation"),
                data: violation_data,
                timestamp: e.ledger().timestamp(),
                verified_by: caller.clone(),
                is_compliant: false,
            };
            
            // Store violation attestation
            let atts_key = (symbol_short!("ATTS"), commitment_id);
            let mut attestations: Vec<Attestation> = e.storage()
                .persistent()
                .get(&atts_key)
                .unwrap_or_else(|| Vec::new(&e));
            attestations.push_back(violation_attestation);
            e.storage().persistent().set(&atts_key, &attestations);
            
            // Emit ViolationDetected event
            e.events().publish(
                (Symbol::new(&e, "ViolationDetected"), commitment_id),
                (drawdown_percent, max_loss_percent, e.ledger().timestamp())
            );
        }
        
        // 7. Create drawdown attestation
        let drawdown_data = Map::new(&e);
        let drawdown_attestation = Attestation {
            commitment_id,
            attestation_type: String::from_str(&e, "drawdown"),
            data: drawdown_data,
            timestamp: e.ledger().timestamp(),
            verified_by: caller.clone(),
            is_compliant: !is_violation,
        };
        
        // Store drawdown attestation
        let atts_key = (symbol_short!("ATTS"), commitment_id);
        let mut attestations: Vec<Attestation> = e.storage()
            .persistent()
            .get(&atts_key)
            .unwrap_or_else(|| Vec::new(&e));
        attestations.push_back(drawdown_attestation);
        e.storage().persistent().set(&atts_key, &attestations);
        
        // 8. Recalculate compliance score
        metrics.compliance_score = Self::calculate_compliance_score(e.clone(), commitment_id);
        
        // 9. Update last attestation timestamp
        metrics.last_attestation = e.ledger().timestamp();
        
        // 10. Store updated health metrics
        Self::store_health_metrics(&e, &metrics);
        
        // 11. Emit DrawdownRecorded event
        e.events().publish(
            (symbol_short!("Drawdown"), commitment_id),
            (current_value, drawdown_percent, e.ledger().timestamp())
        );
    }

    /// Calculate compliance score (0-100)
    pub fn calculate_compliance_score(e: Env, commitment_id: u32) -> u32 {
        let commitment_core: Address = e.storage()
            .instance()
            .get(&symbol_short!("CORE"))
            .unwrap();
        
        let mut args = Vec::new(&e);
        args.push_back(commitment_id.into_val(&e));
        let commitment_val: Val = e.invoke_contract(
            &commitment_core,
            &Symbol::new(&e, "get_commitment"),
            args,
        );
        
        let commitment: Commitment = commitment_val.try_into_val(&e).unwrap();
        let attestations = Self::get_attestations(e.clone(), commitment_id);

        let mut score: i32 = 100;
        
        let violation_count = attestations.iter()
            .filter(|att| !att.is_compliant || att.attestation_type == String::from_str(&e, "violation"))
            .count() as i32;
        score = score.checked_sub(violation_count.checked_mul(20).unwrap_or(0)).unwrap_or(0);
        
        let initial_value = commitment.amount;
        let current_value = commitment.current_value;
        let max_loss_percent = commitment.rules.max_loss_percent as i128;
        
        if initial_value > 0 {
            let drawdown_percent = {
                let diff = initial_value.checked_sub(current_value).unwrap_or(0);
                diff.checked_mul(100).unwrap_or(0)
                    .checked_div(initial_value).unwrap_or(0)
            };
            
            if drawdown_percent > max_loss_percent {
                let over_threshold = drawdown_percent.checked_sub(max_loss_percent).unwrap_or(0);
                score = score.checked_sub(over_threshold as i32).unwrap_or(0);
            }
        }
        
        let min_fee_threshold = commitment.rules.min_fee_threshold;
        let fees_key = (symbol_short!("FEES"), commitment_id);
        let total_fees: i128 = e.storage()
            .persistent()
            .get(&fees_key)
            .unwrap_or(0);
        
        if min_fee_threshold > 0 && total_fees > 0 {
            let fee_percent = total_fees.checked_mul(100).unwrap_or(0)
                .checked_div(min_fee_threshold).unwrap_or(0);
            let bonus = if fee_percent > 100 { 100 } else { fee_percent };
            score = score.checked_add(bonus as i32).unwrap_or(100);
        }
        
        let current_time = e.ledger().timestamp();
        let expires_at = commitment.expires_at;
        let created_at = commitment.created_at;
        
        if expires_at > created_at {
            let total_duration = expires_at.checked_sub(created_at).unwrap_or(1);
            let elapsed = current_time.checked_sub(created_at).unwrap_or(0);
            
            let expected_progress = (elapsed as u128)
                .checked_mul(100).unwrap_or(0)
                .checked_div(total_duration as u128).unwrap_or(0);
            
            if expected_progress <= 100 {
                score = score.checked_add(10).unwrap_or(100);
            }
        }
        
        if score < 0 {
            score = 0;
        } else if score > 100 {
            score = 100;
        }
        
        // Emit compliance score update event
        e.events().publish(
            (symbol_short!("ScoreUpd"), commitment_id),
            (score as u32, e.ledger().timestamp()),
        );
        
        score as u32
    }

    /// Set authorized verifier (admin only)
    pub fn set_authorized_verifier(e: Env, verifier: Address) {
        let admin: Address = e.storage().instance().get(&symbol_short!("ADMIN")).unwrap();
        admin.require_auth();
        e.storage().instance().set(&symbol_short!("AUTH_VER"), &verifier);
    }
}

mod tests;
