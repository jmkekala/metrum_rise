//! Serde defaults and numeric deserializers shared by authored economy schema.

use serde::{Deserialize, Deserializer};

pub(super) fn default_duration_days() -> u32 {
    30
}

pub(super) fn default_one() -> f32 {
    1.0
}

pub(super) fn deserialize_u32_from_number<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(number) => {
            if let Some(unsigned) = number.as_u64() {
                return u32::try_from(unsigned)
                    .map_err(|_| serde::de::Error::custom("numeric value exceeds u32 range"));
            }
            if let Some(signed) = number.as_i64() {
                return u32::try_from(signed).map_err(|_| {
                    serde::de::Error::custom("numeric value must be >= 0 and within u32 range")
                });
            }
            if let Some(float) = number.as_f64() {
                if !float.is_finite() || float < 0.0 {
                    return Err(serde::de::Error::custom(
                        "numeric value must be finite and >= 0",
                    ));
                }
                let rounded = float.round();
                if (float - rounded).abs() > f64::EPSILON {
                    return Err(serde::de::Error::custom(
                        "numeric value must be a whole number",
                    ));
                }
                return u32::try_from(rounded as i64)
                    .map_err(|_| serde::de::Error::custom("numeric value exceeds u32 range"));
            }
            Err(serde::de::Error::custom(
                "unsupported numeric representation",
            ))
        }
        serde_json::Value::Null => Ok(0),
        other => Err(serde::de::Error::custom(format!(
            "expected numeric value for u32 field, got {other}"
        ))),
    }
}

pub(super) fn deserialize_u16_from_number<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(number) => {
            if let Some(unsigned) = number.as_u64() {
                return u16::try_from(unsigned)
                    .map_err(|_| serde::de::Error::custom("numeric value exceeds u16 range"));
            }
            if let Some(signed) = number.as_i64() {
                return u16::try_from(signed).map_err(|_| {
                    serde::de::Error::custom("numeric value must be >= 0 and within u16 range")
                });
            }
            if let Some(float) = number.as_f64() {
                if !float.is_finite() || float < 0.0 {
                    return Err(serde::de::Error::custom(
                        "numeric value must be finite and >= 0",
                    ));
                }
                let rounded = float.round();
                if (float - rounded).abs() > f64::EPSILON {
                    return Err(serde::de::Error::custom(
                        "numeric value must be a whole number",
                    ));
                }
                return u16::try_from(rounded as i64)
                    .map_err(|_| serde::de::Error::custom("numeric value exceeds u16 range"));
            }
            Err(serde::de::Error::custom(
                "unsupported numeric representation",
            ))
        }
        serde_json::Value::Null => Ok(0),
        other => Err(serde::de::Error::custom(format!(
            "expected numeric value for u16 field, got {other}"
        ))),
    }
}
