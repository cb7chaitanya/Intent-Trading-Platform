use std::collections::HashMap;
use std::sync::RwLock;

use uuid::Uuid;

use crate::models::bid::SolverBid;
use crate::models::execution::Execution;
use crate::models::fill::Fill;
use crate::models::intent::Intent;

pub struct Storage {
    intents: RwLock<HashMap<Uuid, Intent>>,
    bids: RwLock<HashMap<Uuid, Vec<SolverBid>>>,
    fills: RwLock<HashMap<Uuid, Fill>>,
    executions: RwLock<HashMap<Uuid, Execution>>,
}

impl Storage {
    pub fn new() -> Self {
        Self {
            intents: RwLock::new(HashMap::new()),
            bids: RwLock::new(HashMap::new()),
            fills: RwLock::new(HashMap::new()),
            executions: RwLock::new(HashMap::new()),
        }
    }

    pub fn insert_intent(&self, intent: Intent) {
        self.intents.write().unwrap().insert(intent.id, intent);
    }

    pub fn get_intent(&self, id: &Uuid) -> Option<Intent> {
        self.intents.read().unwrap().get(id).cloned()
    }

    pub fn list_intents(&self) -> Vec<Intent> {
        self.intents.read().unwrap().values().cloned().collect()
    }

    pub fn update_intent(&self, intent: Intent) {
        self.intents.write().unwrap().insert(intent.id, intent);
    }

    pub fn insert_bid(&self, bid: SolverBid) {
        self.bids
            .write()
            .unwrap()
            .entry(bid.intent_id)
            .or_default()
            .push(bid);
    }

    pub fn get_bids(&self, intent_id: &Uuid) -> Vec<SolverBid> {
        self.bids
            .read()
            .unwrap()
            .get(intent_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn insert_fill(&self, fill: Fill) {
        self.fills.write().unwrap().insert(fill.intent_id, fill);
    }

    pub fn get_fill(&self, intent_id: &Uuid) -> Option<Fill> {
        self.fills.read().unwrap().get(intent_id).cloned()
    }

    pub fn insert_execution(&self, execution: Execution) {
        self.executions
            .write()
            .unwrap()
            .insert(execution.intent_id, execution);
    }

    pub fn get_execution(&self, intent_id: &Uuid) -> Option<Execution> {
        self.executions.read().unwrap().get(intent_id).cloned()
    }

    pub fn update_execution(&self, execution: Execution) {
        self.executions
            .write()
            .unwrap()
            .insert(execution.intent_id, execution);
    }
}
