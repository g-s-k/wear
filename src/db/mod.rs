use {
    super::{Item, SortItems},
    anyhow::Context,
    chrono::{DateTime, Utc},
    sqlx::{
        prelude::*,
        sqlite::{SqlitePool, SqliteRow},
    },
    std::{
        ffi::OsString,
        fmt::{self, Display},
        path::PathBuf,
        time::Instant,
    },
    tokio::fs,
};

type ExecResult = sqlx::Result<u64>;

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

#[derive(Debug)]
pub enum ConnectionError {
    Utf8(OsString),
}

impl std::error::Error for ConnectionError {}

impl Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Utf8(s) => write!(
                f,
                "Cannot convert the following raw path to UTF-8: {}",
                s.to_string_lossy()
            ),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Connection(SqlitePool);

impl Connection {
    pub(crate) async fn new(data_path: Option<PathBuf>) -> anyhow::Result<Self> {
        const PROTOCOL: &str = "sqlite://";

        let (directory, file_name) = super::location::database_file(data_path).await?;
        fs::create_dir_all(&directory).await?;

        let mut file = directory;
        file.push(file_name);

        let mut db_path = OsString::from(PROTOCOL);
        db_path.push(file);

        let string_path = db_path
            .into_string()
            .map_err(ConnectionError::Utf8)?;

        eprintln!("Connecting to database at {}", string_path);
        let before = Instant::now();

        let pool = SqlitePool::new(&string_path).await?;

        eprintln!(
            "Connected to database after {}µs\nConnection pool details: {:#?}",
            before.elapsed().as_micros(),
            pool
        );

        eprintln!("Setting up database...");
        let before = Instant::now();

        pool.acquire()
            .await
            .context("Could not acquire a connection from the pool")?
            .execute(include_str!("./schema.sql"))
            .await
            .context("Failed to apply schema to database")?;

        eprintln!("Done after {}ms", before.elapsed().as_millis());

        Ok(Self(pool))
    }

    pub(crate) async fn close(&self) {
        eprintln!(
            "\r\nClosing database connection [{} connection(s), {} idle]",
            self.0.size(),
            self.0.idle()
        );
        let before = Instant::now();

        self.0.close().await;

        eprintln!(
            "Database connection closed after {}µs",
            before.elapsed().as_micros()
        );
    }

    pub(crate) async fn get_all(
        &self,
        order: &Option<SortItems>,
        mut ascending: bool,
    ) -> sqlx::Result<Vec<Item>> {
        let mut cmd = "SELECT * FROM garments".to_string();

        if let Some(column) = order {
            cmd += " ORDER BY ";
            cmd += match column {
                SortItems::Name => "name",
                SortItems::Count => "count",

                // values stored as datetimes are (to the user) in reverse sort order
                SortItems::Wear => {
                    ascending ^= true;
                    "datetime(wear)"
                }
                SortItems::Wash => {
                    ascending ^= true;
                    "datetime(wash)"
                }
            };
            cmd += if ascending { " ASC" } else { " DESC" };
        }

        sqlx::query_as(&cmd).fetch_all(&self.0).await
    }

    pub(crate) async fn new_item(
        &self,
        Item {
            name,
            description,
            color,
            tags,
            ..
        }: Item,
    ) -> ExecResult {
        sqlx::query("INSERT INTO garments ( name, description, color, tags ) VALUES ( ?, ?, ?, ? )")
            .bind(name)
            .bind(description)
            .bind(color)
            .bind(tags.join(","))
            .execute(&self.0)
            .await
    }

    pub(crate) async fn get_item(&self, item_id: usize) -> sqlx::Result<Item> {
        sqlx::query_as("SELECT * FROM garments WHERE id = ?")
            .bind(item_id as i32)
            .fetch_one(&self.0)
            .await
    }

    pub(crate) async fn update_item(
        &self,
        Item {
            id,
            name,
            description,
            color,
            tags,
            ..
        }: Item,
    ) -> ExecResult {
        sqlx::query(
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
        .bind(id as i32)
        .execute(&self.0)
        .await
    }

    pub(crate) async fn delete_item(&self, item_id: usize) -> ExecResult {
        sqlx::query("DELETE FROM garments WHERE id = ?")
            .bind(item_id as i32)
            .execute(&self.0)
            .await
    }

    pub(crate) async fn log_wear(&self, item_id: usize) -> ExecResult {
        sqlx::query(
            "UPDATE garments SET count = count + 1, total = total + 1, wear = ? WHERE id = ?",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(item_id as i32)
        .execute(&self.0)
        .await
    }

    pub(crate) async fn log_wash(&self, item_id: usize) -> ExecResult {
        sqlx::query("UPDATE garments SET count = 0, wash = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(item_id as i32)
            .execute(&self.0)
            .await
    }
}
