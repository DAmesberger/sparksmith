//! The ranking by meaning: a static embedding model's vector of every
//! topic, compared with the question's.
use crate::Topic;
use model2vec_rs::model::StaticModel;

/// Words of a topic's body its vector reads.
const BODY_WORDS: usize = 250;

/// An embedding model, read from its three files.
pub struct Model(StaticModel);

impl Model {
    pub fn read(tokenizer: &[u8], weights: &[u8], config: &[u8]) -> Result<Self, String> {
        StaticModel::from_bytes(tokenizer, weights, config, Some(true))
            .map(Self)
            .map_err(|e| format!("The help search model could not be read: {e}"))
    }

    pub(crate) fn vectors(&self, topics: &[Topic]) -> Vectors {
        let texts: Vec<String> = topics.iter().map(described).collect();
        Vectors {
            model: self.0.clone(),
            rows: self.0.encode_flat(&texts),
        }
    }
}

fn described(topic: &Topic) -> String {
    let body: Vec<&str> = topic.body.split_whitespace().take(BODY_WORDS).collect();
    format!(
        "{}. {} {} {}",
        topic.title,
        topic.summary,
        topic.keywords,
        body.join(" ")
    )
}

pub(crate) struct Vectors {
    model: StaticModel,
    /// One normalised row per topic.
    rows: Vec<f32>,
}

impl Vectors {
    /// The topics of `wanted` closest to `question` with their cosine
    /// similarity, closest first. A question of no known word has none.
    pub(crate) fn ranked(
        &self,
        question: &str,
        limit: usize,
        wanted: &dyn Fn(usize) -> bool,
    ) -> Vec<(usize, f32)> {
        let asked = self.model.encode_single(question);
        if asked.iter().all(|v| *v == 0.0) {
            return Vec::new();
        }
        let mut close: Vec<(usize, f32)> = self
            .rows
            .chunks_exact(self.model.dim().max(1))
            .enumerate()
            .filter(|(i, _)| wanted(*i))
            .map(|(i, row)| (i, row.iter().zip(&asked).map(|(a, b)| a * b).sum()))
            .collect();
        close.sort_by(|a, b| b.1.total_cmp(&a.1));
        close.truncate(limit);
        close
    }
}
