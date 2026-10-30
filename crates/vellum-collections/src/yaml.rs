// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! An order-preserving YAML value for front matter.
//!
//! Row pages may carry keys the schema does not know (`icon`, `cover`,
//! fields from other tools). They are kept as [`Yaml`] and written back in
//! their original order. Parsing and printing go through `serde-saphyr`,
//! a YAML 1.2 parser in safe Rust.

use std::fmt;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap as _, SerializeSeq as _};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A YAML value.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Yaml {
    /// `null` or `~` or nothing.
    #[default]
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// Any other number.
    Float(f64),
    /// A string.
    String(String),
    /// A list.
    Seq(Vec<Self>),
    /// A mapping, in file order.
    Map(Mapping),
}

impl Yaml {
    /// The string, if this is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// Whether this is `null`.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The text of a scalar, used for map keys that are not strings.
    fn key_text(&self) -> String {
        match self {
            Self::Null => String::new(),
            Self::Bool(b) => b.to_string(),
            Self::Int(i) => i.to_string(),
            Self::Float(f) => f.to_string(),
            Self::String(s) => s.clone(),
            Self::Seq(_) | Self::Map(_) => format!("{self:?}"),
        }
    }
}

/// A YAML mapping with string keys, in insertion order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mapping(Vec<(String, Yaml)>);

impl Mapping {
    /// An empty mapping.
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// The value under `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Yaml> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Whether `key` is present.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Sets `key`, keeping its position if it already exists.
    pub fn insert(&mut self, key: impl Into<String>, value: Yaml) {
        let key = key.into();
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => *v = value,
            None => self.0.push((key, value)),
        }
    }

    /// Removes `key`, returning its value.
    pub fn remove(&mut self, key: &str) -> Option<Yaml> {
        let at = self.0.iter().position(|(k, _)| k == key)?;
        Some(self.0.remove(at).1)
    }

    /// The entries in order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Yaml)> {
        self.0.iter().map(|(k, v)| (k, v))
    }

    /// Whether there are no entries.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl IntoIterator for Mapping {
    type Item = (String, Yaml);
    type IntoIter = std::vec::IntoIter<(String, Yaml)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl Serialize for Yaml {
    fn serialize<S: Serializer>(
        &self,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(b) => serializer.serialize_bool(*b),
            Self::Int(i) => serializer.serialize_i64(*i),
            Self::Float(f) => serializer.serialize_f64(*f),
            Self::String(s) => serializer.serialize_str(s),
            Self::Seq(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for i in items {
                    seq.serialize_element(i)?;
                }
                seq.end()
            }
            Self::Map(m) => m.serialize(serializer),
        }
    }
}

impl Serialize for Mapping {
    fn serialize<S: Serializer>(
        &self,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

struct YamlVisitor;

impl<'de> Visitor<'de> for YamlVisitor {
    type Value = Yaml;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any YAML value")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Yaml, E> {
        Ok(Yaml::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Yaml, E> {
        Ok(Yaml::Int(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Yaml, E> {
        Ok(i64::try_from(v).map_or_else(
            |_| {
                // Above i64::MAX; keep it as the nearest float.
                #[allow(clippy::cast_precision_loss)]
                let f = v as f64;
                Yaml::Float(f)
            },
            Yaml::Int,
        ))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Yaml, E> {
        Ok(Yaml::Float(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Yaml, E> {
        Ok(Yaml::String(v.to_owned()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Yaml, E> {
        Ok(Yaml::String(v))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Yaml, E> {
        Ok(Yaml::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Yaml, E> {
        Ok(Yaml::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Yaml, D::Error> {
        Yaml::deserialize(d)
    }

    fn visit_seq<A: SeqAccess<'de>>(
        self,
        mut seq: A,
    ) -> Result<Yaml, A::Error> {
        let mut items = Vec::new();
        while let Some(v) = seq.next_element()? {
            items.push(v);
        }
        Ok(Yaml::Seq(items))
    }

    fn visit_map<A: MapAccess<'de>>(
        self,
        mut map: A,
    ) -> Result<Yaml, A::Error> {
        let mut out = Mapping::new();
        while let Some((k, v)) = map.next_entry::<Yaml, Yaml>()? {
            out.insert(k.key_text(), v);
        }
        Ok(Yaml::Map(out))
    }
}

impl<'de> Deserialize<'de> for Yaml {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(YamlVisitor)
    }
}

/// Parses YAML text into `T`.
///
/// # Errors
///
/// Returns the parser's message.
pub fn from_str<T: serde::de::DeserializeOwned>(
    text: &str,
) -> Result<T, String> {
    serde_saphyr::from_str(text).map_err(|e| e.to_string())
}

/// Prints `value` as block-style YAML ending in a newline.
///
/// # Errors
///
/// Returns the serializer's message.
pub fn to_string<T: Serialize>(value: &T) -> Result<String, String> {
    let mut out = serde_saphyr::to_string(value).map_err(|e| e.to_string())?;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_and_types() {
        let y: Yaml =
            from_str("b: 1\na: [x, 2.5, true, ~]\nc: {d: e}\n").unwrap();
        let Yaml::Map(m) = &y else {
            panic!("not a map")
        };
        let keys: Vec<&String> = m.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["b", "a", "c"]);
        assert_eq!(m.get("b"), Some(&Yaml::Int(1)));
        let again: Yaml = from_str(&to_string(&y).unwrap()).unwrap();
        assert_eq!(again, y);
    }

    #[test]
    fn mapping_edits_keep_position() {
        let mut m = Mapping::new();
        m.insert("a", Yaml::Int(1));
        m.insert("b", Yaml::Int(2));
        m.insert("a", Yaml::Int(3));
        assert_eq!(m.iter().next().map(|(k, _)| k.as_str()), Some("a"));
        assert_eq!(m.remove("a"), Some(Yaml::Int(3)));
        assert!(!m.contains_key("a"));
        assert!(!m.is_empty());
    }
}
