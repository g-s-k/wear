use std::fmt;

use {
    chrono::{DateTime, Utc},
    serde::{de::Visitor, Deserializer, Serializer},
    warp::{filters::reply::WithHeader, http::Uri, Reply},
};

pub fn html_header() -> WithHeader {
    warp::reply::with::header("Content-Type", "text/html")
}

pub fn go_home() -> impl Reply {
    warp::redirect(Uri::from_static("/"))
}

pub fn default_color() -> String {
    "#000000".into()
}

pub fn format_since(dt: DateTime<Utc>) -> String {
    let dur = Utc::now() - dt;

    if dur.num_minutes() < 60 {
        "right now!".into()
    } else if dur.num_hours() < 24 {
        format!("{} hours ago.", dur.num_hours())
    } else if dur.num_days() < 7 {
        format!("{} days ago.", dur.num_days())
    } else if dur.num_weeks() < 7 {
        format!("{} weeks ago.", dur.num_weeks())
    } else {
        "a while ago.".into()
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
            .split(",")
            .map(str::trim)
            .map(ToOwned::to_owned)
            .collect())
    }
}

pub fn split_comma<'a, D: Deserializer<'a>>(d: D) -> Result<Vec<String>, D::Error> {
    d.deserialize_str(StringListVisitor)
}
