use serde::Deserialize;

#[derive(Deserialize)]
pub struct Root {
    pub token: String,
}
