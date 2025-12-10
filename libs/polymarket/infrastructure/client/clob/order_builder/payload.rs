//! API payload builders for order submission

use super::types::SignedOrder;
use super::super::types::OrderType;

/// Build the JSON payload for placing an order
///
/// The field order matches the Python client exactly:
/// {"order": {...}, "owner": "...", "orderType": "..."}
pub fn build_order_payload(
    signed_order: &SignedOrder,
    owner: &str,
    order_type: OrderType,
) -> serde_json::Value {
    // Use a Map with insertion order preserved to match Python's field order
    let mut map = serde_json::Map::new();
    map.insert("order".to_string(), signed_order.to_api_json());
    map.insert("owner".to_string(), serde_json::Value::String(owner.to_string()));
    map.insert("orderType".to_string(), serde_json::Value::String(order_type.as_str().to_string()));
    serde_json::Value::Object(map)
}

/// Build the JSON payload for placing multiple orders
pub fn build_batch_order_payload(
    signed_orders: &[(SignedOrder, OrderType)],
    owner: &str,
) -> serde_json::Value {
    let orders: Vec<serde_json::Value> = signed_orders
        .iter()
        .map(|(order, order_type)| {
            serde_json::json!({
                "order": order.to_api_json(),
                "owner": owner,
                "orderType": order_type.as_str()
            })
        })
        .collect();

    serde_json::json!(orders)
}
