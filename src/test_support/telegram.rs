use serde_json::{Value, json};

pub fn private_chat_info(id: i64, username: Option<&str>, private_forwards: bool) -> Value {
    json!({
        "id": id,
        "type": "private",
        "first_name": "Telegram account name",
        "username": username,
        "has_private_forwards": private_forwards,
        "accepted_gift_types": {
            "unlimited_gifts": false,
            "limited_gifts": false,
            "unique_gifts": false,
            "premium_subscription": false
        },
        "max_reaction_count": 11
    })
}
