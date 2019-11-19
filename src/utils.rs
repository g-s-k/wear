use {
    chrono::{DateTime, Utc},
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
