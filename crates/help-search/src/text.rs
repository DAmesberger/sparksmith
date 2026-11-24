//! The full-text ranking, by Tantivy, in memory.
use crate::Topic;
use tantivy::collector::TopDocs;
use tantivy::directory::RamDirectory;
use tantivy::query::{BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, Query, TermQuery};
use tantivy::schema::{Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, Value};
use tantivy::tokenizer::{
    Language, LowerCaser, RemoveLongFilter, SimpleTokenizer, Stemmer, StopWordFilter, TextAnalyzer,
    TokenStream, TokenizerManager,
};
use tantivy::{IndexReader, ReloadPolicy, SingleSegmentIndexWriter, TantivyDocument, Term};

/// The analyzer topics are indexed with.
const ENGLISH: &str = "help_en";
/// Memory of the one-segment writer; the whole index is about 2 MB.
const WRITER_BYTES: usize = 32 * 1024 * 1024;
/// How much a match in each field counts: title, keywords, summary, body.
const BOOSTS: [f32; 4] = [8.0, 4.0, 3.0, 1.0];
/// Fields a started word or a misspelling is looked up in: title, keywords.
const LOOSE: usize = 2;

pub(crate) struct Text {
    reader: IndexReader,
    fields: [Field; 4],
    number: Field,
    english: TextAnalyzer,
    words: TextAnalyzer,
}

fn words() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(64))
        .filter(LowerCaser)
        .build()
}

fn english() -> Result<TextAnalyzer, String> {
    let stop = StopWordFilter::new(Language::English).ok_or("No English stop words")?;
    Ok(TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(64))
        .filter(LowerCaser)
        .filter(stop)
        .filter(Stemmer::new(Language::English))
        .build())
}

fn tokens(analyzer: &TextAnalyzer, text: &str) -> Vec<String> {
    let mut analyzer = analyzer.clone();
    let mut stream = analyzer.token_stream(text);
    let mut out = Vec::new();
    while let Some(token) = stream.next() {
        out.push(token.text.clone());
    }
    out
}

impl Text {
    pub(crate) fn new(topics: &[Topic]) -> Result<Self, String> {
        let fail = |e: tantivy::TantivyError| format!("The help index could not be built: {e}");
        let mut schema = Schema::builder();
        let indexed = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(ENGLISH)
                .set_index_option(IndexRecordOption::WithFreqs),
        );
        let fields = ["title", "keywords", "summary", "body"]
            .map(|name| schema.add_text_field(name, indexed.clone()));
        let number = schema.add_u64_field("number", tantivy::schema::STORED);
        let english = english()?;
        let tokenizers = TokenizerManager::default();
        tokenizers.register(ENGLISH, english.clone());
        // One segment, written on this thread: no merges and no thread pool.
        let mut writer: SingleSegmentIndexWriter = tantivy::IndexBuilder::new()
            .schema(schema.build())
            .tokenizers(tokenizers)
            .single_segment_index_writer(RamDirectory::create(), WRITER_BYTES)
            .map_err(fail)?;
        for (i, topic) in topics.iter().enumerate() {
            let mut document = TantivyDocument::new();
            let texts = [&topic.title, &topic.keywords, &topic.summary, &topic.body];
            for (field, text) in fields.iter().zip(texts) {
                document.add_text(*field, text);
            }
            document.add_u64(number, i as u64);
            writer.add_document(document).map_err(fail)?;
        }
        let index = writer.finalize().map_err(fail)?;
        Ok(Self {
            reader: index
                .reader_builder()
                .reload_policy(ReloadPolicy::Manual)
                .try_into()
                .map_err(fail)?,
            fields,
            number,
            english,
            words: words(),
        })
    }

    /// The topics of `wanted` that match a word of `question`, best first.
    pub(crate) fn ranked(&self, question: &str, wanted: &dyn Fn(usize) -> bool) -> Vec<usize> {
        let searcher = self.reader.searcher();
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        let mut add = |query: Box<dyn Query>, boost: f32| {
            let query: Box<dyn Query> = Box::new(BoostQuery::new(query, boost));
            clauses.push((Occur::Should, query));
        };
        for word in tokens(&self.english, question) {
            let mut known = false;
            for (field, boost) in self.fields.iter().zip(BOOSTS) {
                let term = Term::from_field_text(*field, &word);
                if searcher.doc_freq(&term).unwrap_or_default() > 0 {
                    known = true;
                    add(
                        Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)),
                        boost,
                    );
                }
            }
            // A word no topic has: the words it may be a misspelling of.
            let distance = match word.chars().count() {
                0..=3 => 0,
                4..=7 => 1,
                _ => 2,
            };
            if !known && distance > 0 {
                for (field, boost) in self.fields.iter().zip(BOOSTS).take(LOOSE) {
                    let term = Term::from_field_text(*field, &word);
                    add(Box::new(FuzzyTermQuery::new(term, distance, true)), boost);
                }
            }
        }
        // The word being typed: what it is the start of.
        if let Some(last) = tokens(&self.words, question).pop()
            && question.ends_with(|c: char| c.is_alphanumeric())
            && last.chars().count() >= 2
        {
            for (field, boost) in self.fields.iter().zip(BOOSTS).take(LOOSE) {
                let term = Term::from_field_text(*field, &last);
                add(Box::new(FuzzyTermQuery::new_prefix(term, 0, true)), boost);
            }
        }
        if clauses.is_empty() {
            return Vec::new();
        }
        let query = BooleanQuery::new(clauses);
        // Every match is read: the wanted kinds may be a small part of them.
        let all = searcher.num_docs().max(1) as usize;
        let Ok(found) = searcher.search(&query, &TopDocs::with_limit(all).order_by_score()) else {
            return Vec::new();
        };
        found
            .into_iter()
            .filter_map(|(_, at)| {
                let document: TantivyDocument = searcher.doc(at).ok()?;
                usize::try_from(document.get_first(self.number)?.as_u64()?).ok()
            })
            .filter(|i| wanted(*i))
            .collect()
    }
}
