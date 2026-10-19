use laserfoundry_doc::ShapeId;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::EngineError;

/// Deserializes command parameters; `null` is treated as `{}` so defaults apply.
pub(crate) fn parse<T: DeserializeOwned>(params: &Value) -> Result<T, EngineError> {
    let v = if params.is_null() { &Value::Object(Default::default()) } else { params };
    serde_json::from_value(v.clone()).map_err(|e| EngineError::BadParams(e.to_string()))
}

pub(crate) fn id_list(ids: &Option<Vec<ShapeId>>, selection: &[ShapeId]) -> Vec<ShapeId> {
    ids.clone().unwrap_or_else(|| selection.to_vec())
}
