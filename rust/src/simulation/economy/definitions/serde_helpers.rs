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
    deserialize_unsigned_from_number(deserializer, "u32")
}

pub(super) fn deserialize_u16_from_number<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_unsigned_from_number(deserializer, "u16")
}

fn deserialize_unsigned_from_number<'de, D, T>(
    deserializer: D,
    type_name: &'static str,
) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: TryFrom<u64> + TryFrom<i64>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(number) => {
            if let Some(unsigned) = number.as_u64() {
                return convert_unsigned(unsigned, type_name);
            }
            if let Some(signed) = number.as_i64() {
                return convert_signed(signed, type_name);
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
                if rounded > i64::MAX as f64 {
                    return Err(serde::de::Error::custom(format!(
                        "numeric value exceeds {type_name} range"
                    )));
                }
                return convert_signed(rounded as i64, type_name);
            }
            Err(serde::de::Error::custom(
                "unsupported numeric representation",
            ))
        }
        serde_json::Value::Null => convert_unsigned(0, type_name),
        other => Err(serde::de::Error::custom(format!(
            "expected numeric value for {type_name} field, got {other}"
        ))),
    }
}

fn convert_unsigned<T, E>(value: u64, type_name: &'static str) -> Result<T, E>
where
    T: TryFrom<u64>,
    E: serde::de::Error,
{
    T::try_from(value).map_err(|_| E::custom(format!("numeric value exceeds {type_name} range")))
}

fn convert_signed<T, E>(value: i64, type_name: &'static str) -> Result<T, E>
where
    T: TryFrom<i64>,
    E: serde::de::Error,
{
    T::try_from(value).map_err(|_| {
        E::custom(format!(
            "numeric value must be >= 0 and within {type_name} range"
        ))
    })
}
