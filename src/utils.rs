use std::{cmp::Ordering, fmt};

use {
    chrono::{DateTime, Utc},
    serde::{de::Visitor, Deserializer, Serializer},
    warp::{http::StatusCode, Reply},
};

pub fn go_home() -> impl Reply {
    warp::reply::with_header(StatusCode::SEE_OTHER, "Location", "/")
}

pub fn default_color() -> String {
    "#000000".into()
}

pub fn compare_optional_datetimes(
    a: &Option<DateTime<Utc>>,
    b: &Option<DateTime<Utc>>,
) -> Ordering {
    match (a, b) {
        (Some(time_a), Some(time_b)) => time_a.partial_cmp(&time_b).unwrap(),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

pub fn join_comma<S: Serializer>(list: &[String], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&list.join(", "))
}

struct StringListVisitor;

impl<'de> Visitor<'de> for StringListVisitor {
    type Value = Vec<String>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a string containing a comma-separated list of values")
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(value
            .split(',')
            .map(str::trim)
            .map(ToOwned::to_owned)
            .collect())
    }
}

pub fn split_comma<'a, D: Deserializer<'a>>(d: D) -> Result<Vec<String>, D::Error> {
    d.deserialize_str(StringListVisitor)
}
