//! Voyage AI embedding provider (voyage-code-3).

use super::{Embedder, normalize};
use crate::error::{FvaError, Result};
use crate::util;
use serde::{Deserialize, Serialize};

pub struct VoyageEmbedder {
    client: reqwest::blocking::Client,
    api_key: String,
    model: String,
    dimensions: usize,
}

#[derive(Serialize)]
struct EmbedRequest {
    input: Vec<String>,
    model: String,
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

impl VoyageEmbedder {
    pub fn new(api_key: String, model: String, dimensions: usize) -> Result<Self> {
        let client = util::http_client(concat!("fva/", env!("CARGO_PKG_VERSION")))?;

        Ok(Self {
            client,
            api_key,
            model,
            dimensions,
        })
    }
}

impl Embedder for VoyageEmbedder {
    fn name(&self) -> &str {
        "voyage"
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let request = EmbedRequest {
            input: texts.to_vec(),
            model: self.model.clone(),
        };

        let response = self
            .client
            .post("https://api.voyageai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .map_err(|e| FvaError::Other(format!("voyage request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            return Err(FvaError::Other(format!(
                "voyage API error {status}: {body}"
            )));
        }

        let body: EmbedResponse = response
            .json()
            .map_err(|e| FvaError::Other(format!("voyage parse error: {e}")))?;

        let vectors: Vec<Vec<f32>> = body
            .data
            .into_iter()
            .map(|d| {
                let mut v = d.embedding;
                if v.len() != self.dimensions && !v.is_empty() {
                    v.truncate(self.dimensions);
                    v.resize(self.dimensions, 0.0);
                }
                normalize(&mut v);
                v
            })
            .collect();

        if vectors.len() != texts.len() {
            return Err(FvaError::Other(format!(
                "voyage returned {} embeddings for {} texts",
                vectors.len(),
                texts.len()
            )));
        }

        Ok(vectors)
    }
}
