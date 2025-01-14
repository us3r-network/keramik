use ceramic_http_client::GetRootSchema;
use rand::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub trait RandomModelInstance {
    fn random() -> Self;
}

#[derive(Deserialize, JsonSchema, Serialize)]
#[schemars(rename_all = "camelCase", deny_unknown_fields)]
pub struct SmallModel {
    creator: String,
    radius: i32,
    red: i32,
    green: i32,
    blue: i32,
}

impl GetRootSchema for SmallModel {}

impl RandomModelInstance for SmallModel {
    fn random() -> Self {
        let mut rng = thread_rng();
        Self {
            creator: "keramik".to_string(),
            radius: rng.gen_range(0..100),
            red: rng.gen_range(0..255),
            green: rng.gen_range(0..255),
            blue: rng.gen_range(0..255),
        }
    }
}

#[derive(Deserialize, JsonSchema, Serialize)]
#[schemars(rename_all = "camelCase", deny_unknown_fields)]
pub struct LargeModel {
    pub creator: String,
    pub name: String,
    pub description: String,
    pub tpe: i64,
}

impl GetRootSchema for LargeModel {}

impl RandomModelInstance for LargeModel {
    fn random() -> Self {
        let mut rng = thread_rng();
        let name: String = (1..100).map(|_| rng.gen::<char>()).collect();
        Self {
            creator: "keramik".to_string(),
            name: format!("keramik-large-model-{}", name),
            description: (1..1_000).map(|_| rng.gen::<char>()).collect(),
            tpe: rng.gen_range(0..100),
        }
    }
}
