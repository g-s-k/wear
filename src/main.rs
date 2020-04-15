#![deny(clippy::all)]

use {
    chrono::{DateTime, Utc},
    chrono_humanize::Humanize,
    handlebars::Handlebars,
    serde::{Deserialize, Serialize},
    serde_json::json,
    sqlx::{
        prelude::*,
        sqlite::{SqlitePool, SqliteRow},
    },
    std::sync::Arc,
    warp::{path, Filter},
};

mod utils;

struct WithTemplate<T: Serialize> {
    name: &'static str,
    value: T,
}

fn render<T>(template: WithTemplate<T>, hbs: Arc<Handlebars>) -> impl warp::Reply
where
    T: Serialize,
{
    warp::reply::html(
        hbs.render(template.name, &template.value)
            .unwrap_or_else(|err| format!("{}", err)),
    )
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Item {
    #[serde(default)]
    id: usize,
    name: String,
    description: String,
    #[serde(default)]
    count: usize,
    #[serde(default)]
    total_count: usize,
    #[serde(default)]
    last_wear: Option<DateTime<Utc>>,
    #[serde(default)]
    last_wash: Option<DateTime<Utc>>,
    #[serde(default = "utils::default_color")]
    color: String,
    #[serde(
        deserialize_with = "utils::split_comma",
        serialize_with = "utils::join_comma"
    )]
    tags: Vec<String>,
}

impl<'c> FromRow<'c, SqliteRow<'c>> for Item {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Item {
            id: row.try_get::<i32, _>("id")? as usize,
            name: row.try_get::<String, _>("name")?,
            description: row.try_get::<String, _>("description")?,
            count: row.try_get::<i32, _>("count")? as usize,
            total_count: row.try_get::<i32, _>("total")? as usize,
            last_wash: row
                .try_get::<Option<&str>, _>("wash")?
                .map(DateTime::parse_from_rfc3339)
                .map(Result::ok)
                .flatten()
                .map(|d| d.with_timezone(&Utc)),
            last_wear: row
                .try_get::<Option<&str>, _>("wear")?
                .map(DateTime::parse_from_rfc3339)
                .map(Result::ok)
                .flatten()
                .map(|d| d.with_timezone(&Utc)),
            color: row.try_get::<String, _>("color")?,
            tags: row
                .try_get::<&str, _>("tags")?
                .split(',')
                .map(ToOwned::to_owned)
                .collect(),
        })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum SortItems {
    Name,
    Count,
    Wear,
    Wash,
}

#[derive(Deserialize)]
struct IndexOpts {
    sort: Option<SortItems>,
    descending: Option<bool>,
}

impl IndexOpts {
    fn get_sort_fn(&self) -> fn(&Item, &Item) -> std::cmp::Ordering {
        match self.sort {
            Some(SortItems::Name) => {
                |Item { name: ref a, .. }, Item { name: ref b, .. }| (a).partial_cmp(b).unwrap()
            }
            Some(SortItems::Count) => {
                |Item { count: a, .. }, Item { count: b, .. }| a.partial_cmp(&b).unwrap()
            }
            Some(SortItems::Wear) => |Item { last_wear: a, .. }, Item { last_wear: b, .. }| {
                utils::compare_optional_datetimes(a, b)
            },
            Some(SortItems::Wash) => {
                |Item { last_wash: a, .. }, Item { last_wash: b, .. }| a.partial_cmp(b).unwrap()
            }
            None => |Item { id: a, .. }, Item { id: b, .. }| a.partial_cmp(b).unwrap(),
        }
    }
}

async fn home_page(
    params: IndexOpts,
    pool: SqlitePool,
) -> Result<WithTemplate<serde_json::Value>, warp::Rejection> {
    let items = match sqlx::query_as("SELECT * FROM garments")
        .fetch_all(&pool)
        .await
    {
        Ok(mut i) => {
            if params.sort.is_some() {
                i.sort_unstable_by(params.get_sort_fn());
            }

            if let Some(true) = params.descending {
                i.reverse();
            }

            i.iter()
                .map(
                    |Item {
                         id,
                         name,
                         description,
                         count,
                         total_count,
                         last_wear,
                         last_wash,
                         color,
                         tags,
                     }| {
                        json!({
                            "key": id,
                            "name": name,
                            "description": description,
                            "count": count,
                            "totalCount": total_count,
                            "hasWear": last_wear.is_some(),
                            "wear": last_wear,
                            "wearFmt": last_wear.map(|t| (t - Utc::now()).humanize()),
                            "hasWash": last_wash.is_some(),
                            "wash": last_wash,
                            "washFmt": last_wash.map(|t| (t - Utc::now()).humanize()),
                            "color": color,
                            "tags": tags.join(", "),
                        })
                    },
                )
                .collect::<Vec<_>>()
        }
        Err(e) => {
            eprintln!("request for index: could not retrieve collection: {}", e);
            Vec::new()
        }
    };

    Ok(WithTemplate {
        name: "index",
        value: json!({
            "items": items,
            "numItems": items.len(),
            "sort": params.sort,
            "descending": params.descending,
        }),
    })
}

async fn handle_post_item(s: SqlitePool, item: Item) -> Result<impl warp::Reply, warp::Rejection> {
    match sqlx::query(
        "INSERT INTO garments ( name, description, color, tags ) VALUES ( ?, ?, ?, ? )",
    )
    .bind(&item.name)
    .bind(item.description)
    .bind(item.color)
    .bind(item.tags.join(","))
    .execute(&s)
    .await
    {
        Ok(_) => Ok(utils::go_home()),
        Err(e) => {
            eprintln!("could not insert new item ({}): {}", item.name, e);
            Err(warp::reject::not_found())
        }
    }
}

async fn handle_update_item(
    i: usize,
    s: SqlitePool,
    Item {
        name,
        description,
        color,
        tags,
        ..
    }: Item,
) -> Result<impl warp::Reply, warp::Rejection> {
    match sqlx::query(
        r#"
            UPDATE garments
            SET color = ?, name = ?, description = ?, tags = ?
            WHERE id = ?
        "#,
    )
    .bind(color)
    .bind(name)
    .bind(description)
    .bind(tags.join(","))
    .bind(i as i32)
    .execute(&s)
    .await
    {
        Ok(_) => Ok(utils::go_home()),
        Err(e) => {
            eprintln!("{}", e);
            Err(warp::reject::not_found())
        }
    }
}

async fn handle_increment(i: usize, s: SqlitePool) -> Result<impl warp::Reply, warp::Rejection> {
    match sqlx::query(
        "UPDATE garments SET count = count + 1, total = total + 1, wear = ? WHERE id = ?",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(i as i32)
    .execute(&s)
    .await
    {
        Ok(_) => Ok(utils::go_home()),
        Err(e) => {
            eprintln!("{}", e);
            Err(warp::reject::not_found())
        }
    }
}

async fn handle_reset(i: usize, s: SqlitePool) -> Result<impl warp::Reply, warp::Rejection> {
    match sqlx::query("UPDATE garments SET count = 0, wash = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(i as i32)
        .execute(&s)
        .await
    {
        Ok(_) => Ok(utils::go_home()),
        Err(e) => {
            eprintln!("{}", e);
            Err(warp::reject::not_found())
        }
    }
}

async fn handle_delete_item(i: usize, s: SqlitePool) -> Result<impl warp::Reply, warp::Rejection> {
    sqlx::query("DELETE FROM garments WHERE id = ?")
        .bind(i as i32)
        .execute(&s)
        .await
        .map_err(|_| warp::reject::not_found())?;
    Ok(utils::go_home())
}

async fn handle_edit_form(
    i: usize,
    s: SqlitePool,
) -> Result<WithTemplate<serde_json::Value>, warp::Rejection> {
    match sqlx::query_as("SELECT * FROM garments WHERE id = ?")
        .bind(i as i32)
        .fetch_one(&s)
        .await
    {
        Ok(Item {
            id,
            name,
            description,
            color,
            tags,
            ..
        }) => Ok(WithTemplate {
            name: "edit",
            value: json!({
                "edit": true,
                "key": id,
                "name": name,
                "description": description,
                "color": color,
                "tags": tags.join(", "),
            }),
        }),
        Err(e) => {
            eprintln!("{}", e);
            Err(warp::reject::not_found())
        }
    }
}

#[tokio::main]
async fn main() {
    let mut hb = Handlebars::new();
    hb.register_template_string("index", include_str!("./static/index.hbs"))
        .unwrap();
    hb.register_partial("nav", include_str!("./static/nav.hbs"))
        .unwrap();
    hb.register_partial("form", include_str!("./static/form.hbs"))
        .unwrap();
    hb.register_template_string("new", include_str!("./static/new.hbs"))
        .unwrap();
    hb.register_template_string("edit", include_str!("./static/edit.hbs"))
        .unwrap();
    let hb = Arc::new(hb);
    let hbars = move |with_template| render(with_template, hb.clone());

    let db_path = format!(
        "sqlite://{}/data.db",
        std::env::current_dir().unwrap().to_string_lossy()
    );
    eprintln!("Connecting to database at {}", db_path);
    let state = SqlitePool::new(&db_path).await.unwrap();
    eprintln!(
        "Connected to database. Connection pool details: {:#?}",
        state
    );
    let with_state = warp::any().map(move || state.clone());

    let index = warp::get()
        .and(path::end())
        .and(warp::query::query())
        .and(with_state.clone())
        .and_then(home_page)
        .map(hbars.clone());

    let css = path("styles.css").and(path::end()).map(|| {
        warp::reply::with_header(
            include_str!("./static/styles.css"),
            "Content-Type",
            "text/css",
        )
    });

    let new = warp::get()
        .and(warp::path("new"))
        .and(path::end())
        .map(|| WithTemplate {
            name: "new",
            value: json!({}),
        })
        .map(hbars.clone());

    let post_item = warp::post()
        .and(path::end())
        .and(with_state.clone())
        .and(warp::body::content_length_limit(1024 * 32))
        .and(warp::body::form())
        .and_then(handle_post_item);

    let edit_item = warp::get()
        .and(path::param())
        .and(path::end())
        .and(with_state.clone())
        .and_then(handle_edit_form)
        .map(hbars);

    let update_item = warp::post()
        .and(path::param())
        .and(path::end())
        .and(with_state.clone())
        .and(warp::body::content_length_limit(1024 * 32))
        .and(warp::body::form())
        .and_then(handle_update_item);

    let increment_item = warp::post()
        .and(path::param())
        .and(warp::path("increment"))
        .and(path::end())
        .and(with_state.clone())
        .and_then(handle_increment);

    let reset_item = warp::post()
        .and(path::param())
        .and(warp::path("reset"))
        .and(path::end())
        .and(with_state.clone())
        .and_then(handle_reset);

    let delete_item = warp::post()
        .and(path::param())
        .and(path("remove"))
        .and(path::end())
        .and(with_state)
        .and_then(handle_delete_item);

    let router = index
        .or(css)
        .or(warp::path("item").and(
            post_item
                .or(new)
                .or(edit_item)
                .or(update_item)
                .or(increment_item)
                .or(reset_item)
                .or(delete_item),
        ))
        .with(warp::log("wear"));

    warp::serve(router).run(([0, 0, 0, 0], 3000)).await;
}
