// SPDX-License-Identifier: Apache-2.0
use crate::{digest, Error, Operation, Result, PLAN_LIFETIME_SECONDS};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Caller {
    pub unique_name: String,
    pub uid: u32,
    pub pid: u32,
    pub process_start: u64,
    pub session: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalPlan {
    pub version: u32,
    pub id: String,
    pub caller: Caller,
    pub action_id: String,
    pub operation: Operation,
    pub argument_digest: String,
    pub state: Value,
    pub issued_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prepared {
    pub plan: CanonicalPlan,
    pub digest: String,
}

#[derive(Default)]
pub struct PlanStore {
    plans: HashMap<String, Prepared>,
}

impl PlanStore {
    pub fn prepare(
        &mut self,
        caller: Caller,
        operation: Operation,
        state: Value,
        now: u64,
    ) -> Result<Prepared> {
        operation.validate()?;
        self.plans.retain(|_, p| now < p.plan.expires_at);
        if self.plans.len() >= 256
            || self
                .plans
                .values()
                .filter(|p| p.plan.caller.unique_name == caller.unique_name)
                .count()
                >= 8
        {
            return Err(Error("too_many_pending_plans"));
        }
        let mut nonce = [0_u8; 32];
        OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| Error("entropy_unavailable"))?;
        let id = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let plan = CanonicalPlan {
            version: 1,
            id,
            caller,
            action_id: operation.action_id().into(),
            argument_digest: digest(&operation)?,
            operation,
            state,
            issued_at: now,
            expires_at: now
                .checked_add(PLAN_LIFETIME_SECONDS)
                .ok_or(Error("invalid_time"))?,
        };
        let prepared = Prepared {
            digest: digest(&plan)?,
            plan,
        };
        self.plans
            .insert(prepared.plan.id.clone(), prepared.clone());
        Ok(prepared)
    }

    /// Consume before authentication: cancellation, denial, and failed execution
    /// all require a new preview. A second concurrent Apply cannot reuse it.
    pub fn consume(
        &mut self,
        id: &str,
        expected_digest: &str,
        caller: &Caller,
        now: u64,
    ) -> Result<CanonicalPlan> {
        if id.len() != 64 || expected_digest.len() != 64 {
            return Err(Error("invalid_plan_reference"));
        }
        let prepared = self.plans.get(id).ok_or(Error("unknown_or_used_plan"))?;
        // A different caller cannot invalidate another caller's pending plan.
        if &prepared.plan.caller != caller {
            return Err(Error("caller_changed"));
        }
        let prepared = self.plans.remove(id).ok_or(Error("unknown_or_used_plan"))?;
        if now < prepared.plan.issued_at || now >= prepared.plan.expires_at {
            return Err(Error("plan_expired"));
        }
        if prepared.digest != expected_digest {
            return Err(Error("plan_digest_changed"));
        }
        Ok(prepared.plan)
    }
}

pub fn revalidate(plan: &CanonicalPlan, caller: &Caller, state: &Value, now: u64) -> Result<()> {
    if &plan.caller != caller {
        return Err(Error("caller_changed"));
    }
    if now < plan.issued_at || now >= plan.expires_at {
        return Err(Error("plan_expired"));
    }
    if &plan.state != state {
        return Err(Error("state_changed_prepare_again"));
    }
    if plan.action_id != plan.operation.action_id()
        || plan.argument_digest != digest(&plan.operation)?
    {
        return Err(Error("plan_changed"));
    }
    plan.operation.validate()
}
