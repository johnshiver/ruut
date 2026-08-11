use std::path::Path;

use redb::Database;
use sqlparser::ast::Statement;
use sqlparser::dialect::PostgreSqlDialect;

use crate::tuple::TupleCodecError;

pub const PRIMARY_TABLE_SUFFIX: &str = "primary";
pub const HISTORY_TABLE_SUFFIX: &str = "history";
pub const INDEX_TABLE_PREFIX: &str = "idx";

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("optimistic concurrency check failed for `{table}` on key {primary_key:?}: expected version {expected}, found {actual}")]
    ConcurrencyConflict {
        table: String,
        primary_key: Vec<u8>,
        expected: u32,
        actual: u32,
    },
    #[error(transparent)]
    Database(#[from] redb::DatabaseError),
    #[error(transparent)]
    Transaction(#[from] redb::TransactionError),
    #[error(transparent)]
    Sql(#[from] sqlparser::parser::ParserError),
    #[error(transparent)]
    Tuple(#[from] TupleCodecError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnSchema {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSchema {
    pub name: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSchema {
    pub name: String,
    pub primary_key_columns: Vec<String>,
    pub columns: Vec<ColumnSchema>,
    pub indexes: Vec<IndexSchema>,
    pub save_history: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableLayout {
    pub primary_table: String,
    pub history_table: Option<String>,
    pub index_tables: Vec<String>,
}

impl TableLayout {
    pub fn from_schema(schema: &TableSchema) -> Self {
        let primary_table = format!("{}_{}", schema.name, PRIMARY_TABLE_SUFFIX);
        let history_table = schema
            .save_history
            .then(|| format!("{}_{}", schema.name, HISTORY_TABLE_SUFFIX));
        let index_tables = schema
            .indexes
            .iter()
            .map(|index| format!("{}_{}_{}", schema.name, INDEX_TABLE_PREFIX, index.name))
            .collect();

        Self {
            primary_table,
            history_table,
            index_tables,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    pub table_name: String,
    pub primary_key: Vec<u8>,
    pub expected_version: u32,
    pub new_tuple: Vec<u8>,
}

pub struct StorageEngine {
    db: Database,
    dialect: PostgreSqlDialect,
}

impl StorageEngine {
    pub fn create(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Ok(Self {
            db: Database::create(path)?,
            dialect: PostgreSqlDialect {},
        })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Ok(Self {
            db: Database::open(path)?,
            dialect: PostgreSqlDialect {},
        })
    }

    pub fn dialect(&self) -> &PostgreSqlDialect {
        &self.dialect
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    pub fn begin_write(&self, tx_id: u64) -> Result<WriteTransaction, StorageError> {
        Ok(WriteTransaction {
            inner: self.db.begin_write()?,
            tx_id,
        })
    }

    pub fn parse_sql(&self, sql: &str) -> Result<Vec<Statement>, StorageError> {
        Ok(sqlparser::parser::Parser::parse_sql(&self.dialect, sql)?)
    }

    pub fn create_table(&self, schema: &TableSchema) -> Result<TableLayout, StorageError> {
        let _ = schema;
        todo!("provision redb primary/history/index tables")
    }
}

pub struct WriteTransaction {
    inner: redb::WriteTransaction,
    tx_id: u64,
}

impl WriteTransaction {
    pub fn redb(&self) -> &redb::WriteTransaction {
        &self.inner
    }

    pub fn tx_id(&self) -> u64 {
        self.tx_id
    }

    pub fn create_table(&mut self, schema: &TableSchema) -> Result<TableLayout, StorageError> {
        let _ = schema;
        todo!("create primary/history/index tables inside a redb write transaction")
    }

    pub fn update_with_occ(&mut self, plan: &UpdatePlan) -> Result<(), StorageError> {
        let _ = plan;
        todo!("read current tuple, validate sys_version, copy old bytes to history, write new tuple, update indexes, and commit atomically")
    }

    pub fn commit(self) -> Result<(), StorageError> {
        todo!("commit wrapped redb write transaction atomically")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_layout_uses_sql_over_kv_naming_convention() {
        let schema = TableSchema {
            name: "users".to_string(),
            primary_key_columns: vec!["id".to_string()],
            columns: vec![ColumnSchema {
                name: "id".to_string(),
            }],
            indexes: vec![IndexSchema {
                name: "email".to_string(),
                columns: vec!["email".to_string()],
            }],
            save_history: true,
        };

        let layout = TableLayout::from_schema(&schema);

        assert_eq!(layout.primary_table, "users_primary");
        assert_eq!(layout.history_table.as_deref(), Some("users_history"));
        assert_eq!(layout.index_tables, vec!["users_idx_email"]);
    }
}
