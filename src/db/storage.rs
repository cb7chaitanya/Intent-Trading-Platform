use std::collections::HashMap;
use std::sync::RwLock;

use uuid::Uuid;

use crate::models::bid::SolverBid;
use crate::models::intent::Intent;

pub struct Storage {
    intents: RwLock<HashMap<Uuid, Intent>>,
    bids: RwLock<HashMap<Uuid, Vec<SolverBid>>>,
}

impl Storage {
    pub fn new() -> Self {
        Self {
            intents: RwLock::new(HashMap::new()),
            bids: RwLock::new(HashMap::new()),
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
}
