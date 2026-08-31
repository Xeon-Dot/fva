//! Native LanceDB vector store.
//!
//! Data persists automatically in the Lance format — `persist()` is a no-op.

use std::sync::Arc;

use arrow_array::{FixedSizeListArray, Float32Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{Connection, DistanceType, Table};

use super::{VectorHit, VectorStats};
use crate::error::{FvaError, Result};
use crate::indexer::chunker::CodeChunk;
use crate::util::truncate_preview;

const TABLE_NAME: &str = "chunks";

pub struct LanceDbVectorStore {
    table: Table,
    dimensions: usize,
}

/// The "vector" column must be the only vector column for nearest_to() to
/// auto-discover it; we name it "vector".
fn schema(dimensions: usize) -> Schema {
    let dim = dimensions as i32;
    Schema::new(vec![
        Field::new("chunk_id", DataType::Utf8, false),
        Field::new("relative_path", DataType::Utf8, false),
        Field::new("symbol_name", DataType::Utf8, false),
        Field::new("symbol_kind", DataType::Utf8, false),
        Field::new("language", DataType::Utf8, false),
        Field::new("start_line", DataType::Int64, false),
        Field::new("end_line", DataType::Int64, false),
        Field::new("content_preview", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim),
            true,
        ),
    ])
}

fn vector_dims(schema: &Schema) -> Option<usize> {
    match schema.field_with_name("vector").ok()?.data_type() {
        DataType::FixedSizeList(_, n) => Some(*n as usize),
        _ => None,
    }
}

fn escape_sql_literal(s: &str) -> String {
    s.replace('\'', "''")
}

impl LanceDbVectorStore {
    pub async fn open(path: std::path::PathBuf, dimensions: usize) -> Result<Self> {
        std::fs::create_dir_all(&path)?;
        let uri = path
            .to_str()
            .ok_or_else(|| FvaError::Other(format!("non-utf8 vector path: {}", path.display())))?;
        let conn: Connection = lancedb::connect(uri)
            .execute()
            .await
            .map_err(|e| FvaError::Other(format!("lancedb connect: {e}")))?;

        let table = match conn.open_table(TABLE_NAME).execute().await {
            Ok(t) => {
                let tbl_schema = t
                    .schema()
                    .await
                    .map_err(|e| FvaError::Other(format!("lancedb schema: {e}")))?;
                if vector_dims(&tbl_schema) != Some(dimensions) {
                    tracing::warn!(
                        "vector dimensions changed, dropping lancedb table — re-index required"
                    );
                    conn.drop_table(TABLE_NAME, &[])
                        .await
                        .map_err(|e| FvaError::Other(format!("lancedb drop_table: {e}")))?;
                    conn.create_empty_table(TABLE_NAME, Arc::new(schema(dimensions)))
                        .execute()
                        .await
                        .map_err(|e| FvaError::Other(format!("lancedb create: {e}")))?
                } else {
                    t
                }
            }
            Err(_) => conn
                .create_empty_table(TABLE_NAME, Arc::new(schema(dimensions)))
                .execute()
                .await
                .map_err(|e| FvaError::Other(format!("lancedb create: {e}")))?,
        };

        Ok(Self { table, dimensions })
    }
}

impl LanceDbVectorStore {
    pub async fn upsert_chunks(
        &self,
        chunks: &[CodeChunk],
        vectors: &[Vec<f32>],
    ) -> Result<()> {
        if chunks.len() != vectors.len() {
            return Err(FvaError::Other(format!(
                "chunk/vector count mismatch: {} vs {}",
                chunks.len(),
                vectors.len()
            )));
        }
        if let Some(path) = chunks.first().map(|c| c.relative_path.as_str()) {
            self.remove_file(path).await?;
        }
        let batch = RecordBatch::try_new(
            Arc::new(schema(self.dimensions)),
            vec![
                Arc::new(StringArray::from(
                    chunks.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    chunks
                        .iter()
                        .map(|c| c.relative_path.clone())
                        .collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    chunks
                        .iter()
                        .map(|c| c.symbol_name.clone())
                        .collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    chunks
                        .iter()
                        .map(|c| c.symbol_kind.clone())
                        .collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    chunks
                        .iter()
                        .map(|c| c.language.clone())
                        .collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from(
                    chunks
                        .iter()
                        .map(|c| c.start_line as i64)
                        .collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from(
                    chunks.iter().map(|c| c.end_line as i64).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    chunks
                        .iter()
                        .map(|c| truncate_preview(&c.content, 200))
                        .collect::<Vec<_>>(),
                )),
                Arc::new(FixedSizeListArray::from_iter_primitive::<
                    arrow_array::types::Float32Type,
                    _,
                    _,
                >(
                    vectors
                        .iter()
                        .map(|v| Some(v.iter().map(|x| Some(*x)).collect::<Vec<_>>()))
                        .collect::<Vec<_>>(),
                    self.dimensions as i32,
                )),
            ],
        )
        .map_err(|e| FvaError::Other(format!("arrow batch: {e}")))?;

        self.table
            .add(batch)
            .execute()
            .await
            .map_err(|e| FvaError::Other(format!("lancedb add: {e}")))?;
        Ok(())
    }

    pub(crate) async fn remove_file(&self, relative_path: &str) -> Result<()> {
        let sql = format!(
            "relative_path = '{}'",
            escape_sql_literal(relative_path)
        );
        self.table
            .delete(&sql)
            .await
            .map_err(|e| FvaError::Other(format!("lancedb delete: {e}")))?;
        Ok(())
    }

    pub async fn search(
        &self,
        query_vector: &[f32],
        limit: usize,
    ) -> Result<Vec<VectorHit>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let stream = self
            .table
            .query()
            .nearest_to(query_vector)
            .map_err(|e| FvaError::Other(format!("lancedb query: {e}")))?
            .distance_type(DistanceType::Cosine)
            .limit(limit)
            .execute()
            .await
            .map_err(|e| FvaError::Other(format!("lancedb search: {e}")))?;

        let batches: Vec<RecordBatch> = stream
            .try_collect()
            .await
            .map_err(|e| FvaError::Other(format!("lancedb stream: {e}")))?;

        let mut hits = Vec::new();
        for batch in batches {
            let schema = batch.schema();
            let idx = |name: &str| -> Result<usize> {
                schema
                    .index_of(name)
                    .map_err(|e| FvaError::Other(format!("column {name}: {e}")))
            };
            let (i_chunk, i_path, i_sym, i_kind, i_lang, i_start, i_end, i_prev, i_dist) = (
                idx("chunk_id")?,
                idx("relative_path")?,
                idx("symbol_name")?,
                idx("symbol_kind")?,
                idx("language")?,
                idx("start_line")?,
                idx("end_line")?,
                idx("content_preview")?,
                idx("_distance")?,
            );
            for row in 0..batch.num_rows() {
                let col = |i: usize| -> Result<&StringArray> {
                    batch
                        .column(i)
                        .as_any()
                        .downcast_ref::<StringArray>()
                        .ok_or_else(|| FvaError::Other("column not utf8".to_string()))
                };
                hits.push(VectorHit {
                    chunk_id: col(i_chunk)?.value(row).to_string(),
                    relative_path: col(i_path)?.value(row).to_string(),
                    symbol_name: col(i_sym)?.value(row).to_string(),
                    symbol_kind: col(i_kind)?.value(row).to_string(),
                    language: col(i_lang)?.value(row).to_string(),
                    start_line: batch
                        .column(i_start)
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .ok_or_else(|| FvaError::Other("start_line not int".to_string()))?
                        .value(row) as usize,
                    end_line: batch
                        .column(i_end)
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .ok_or_else(|| FvaError::Other("end_line not int".to_string()))?
                        .value(row) as usize,
                    content_preview: col(i_prev)?.value(row).to_string(),
                    score: 1.0
                        - batch
                            .column(i_dist)
                            .as_any()
                            .downcast_ref::<Float32Array>()
                            .ok_or_else(|| FvaError::Other("distance not float".to_string()))?
                            .value(row),
                });
            }
        }
        Ok(hits)
    }

    pub fn stats(&self) -> VectorStats {
        let table = self.table.clone();
        let total = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("lancedb stats runtime");
            rt.block_on(table.count_rows(None))
        })
        .join()
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_else(|| {
            tracing::warn!("lancedb count_rows failed");
            0
        });
        VectorStats {
            total_vectors: total,
            dimensions: self.dimensions,
        }
    }

    pub fn persist(&self) -> Result<()> {
        Ok(())
    }
}
