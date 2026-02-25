use anyhow::Result;
use lindera::dictionary::{DictionaryKind, load_dictionary_from_kind};
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera_tantivy::tokenizer::LinderaTokenizer;
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{Index, IndexWriter, ReloadPolicy, doc};

const KOREAN_TOKENIZER_NAME: &str = "korean";

/// Build a Korean morphological tokenizer using lindera ko-dic.
///
/// This tokenizer splits agglutinative Korean text into morphemes:
///   "한국어로" → ["한국어", "로"]
///   "프로그래밍을" → ["프로그래밍", "을"]
fn build_korean_tokenizer() -> LinderaTokenizer {
    let dictionary =
        load_dictionary_from_kind(DictionaryKind::KoDic).expect("Failed to load ko-dic dictionary");
    let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
    LinderaTokenizer::from_segmenter(segmenter)
}

/// Register the Korean tokenizer on a tantivy Index.
fn register_korean_tokenizer(index: &Index) {
    index
        .tokenizers()
        .register(KOREAN_TOKENIZER_NAME, build_korean_tokenizer());
}

/// BM25 full-text search index using Tantivy with Korean morphological analysis
pub struct Bm25Index {
    index: Index,
    _schema: Schema,
    id_field: Field,
    content_field: Field,
    title_field: Field,
}

impl Bm25Index {
    /// Create a new BM25 index at the given directory
    pub fn new(index_dir: impl AsRef<Path>) -> Result<Self> {
        let mut schema_builder = Schema::builder();

        let id_field = schema_builder.add_text_field("id", STRING | STORED);
        let content_field = schema_builder.add_text_field(
            "content",
            TextOptions::default().set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer(KOREAN_TOKENIZER_NAME)
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            ),
        );
        let title_field = schema_builder.add_text_field(
            "title",
            TextOptions::default().set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer(KOREAN_TOKENIZER_NAME)
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            ),
        );

        let schema = schema_builder.build();
        let index_path = index_dir.as_ref();
        std::fs::create_dir_all(index_path)?;
        let index = Index::open_or_create(
            tantivy::directory::MmapDirectory::open(index_path)?,
            schema.clone(),
        )?;

        register_korean_tokenizer(&index);

        Ok(Self {
            index,
            _schema: schema,
            id_field,
            content_field,
            title_field,
        })
    }

    /// Create an in-memory index (for testing)
    pub fn in_memory() -> Result<Self> {
        let mut schema_builder = Schema::builder();

        let id_field = schema_builder.add_text_field("id", STRING | STORED);
        let content_field = schema_builder.add_text_field(
            "content",
            TextOptions::default().set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer(KOREAN_TOKENIZER_NAME)
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            ),
        );
        let title_field = schema_builder.add_text_field(
            "title",
            TextOptions::default().set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer(KOREAN_TOKENIZER_NAME)
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            ),
        );

        let schema = schema_builder.build();
        let index = Index::create_in_ram(schema.clone());

        register_korean_tokenizer(&index);

        Ok(Self {
            index,
            _schema: schema,
            id_field,
            content_field,
            title_field,
        })
    }

    /// Index a document
    pub fn add(&self, id: &str, title: &str, content: &str) -> Result<()> {
        let mut writer: IndexWriter = self.index.writer(50_000_000)?;
        writer.add_document(doc!(
            self.id_field => id,
            self.title_field => title,
            self.content_field => content,
        ))?;
        writer.commit()?;
        Ok(())
    }

    /// Search for documents matching the query
    pub fn search(&self, query_str: &str, limit: usize) -> Result<Vec<(String, f32)>> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        let searcher = reader.searcher();

        let query_parser =
            QueryParser::for_index(&self.index, vec![self.content_field, self.title_field]);
        let query = query_parser.parse_query(query_str)?;

        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::new();
        for (score, doc_address) in top_docs {
            let doc = searcher.doc::<TantivyDocument>(doc_address)?;
            if let Some(id_value) = doc.get_first(self.id_field)
                && let Some(id) = id_value.as_str()
            {
                results.push((id.to_string(), score));
            }
        }

        Ok(results)
    }

    /// Remove a document by ID
    pub fn remove(&self, id: &str) -> Result<()> {
        let mut writer: IndexWriter = self.index.writer(50_000_000)?;
        let term = tantivy::Term::from_field_text(self.id_field, id);
        writer.delete_term(term);
        writer.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bm25_index_search() {
        let index = Bm25Index::in_memory().unwrap();

        index
            .add(
                "1",
                "Rust 프로그래밍",
                "Rust는 안전하고 빠른 시스템 프로그래밍 언어입니다",
            )
            .unwrap();
        index
            .add(
                "2",
                "Python 프로그래밍",
                "Python은 간결하고 읽기 쉬운 스크립트 언어입니다",
            )
            .unwrap();

        let results = index.search("Rust 안전", 5).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "1");
    }

    #[test]
    fn test_korean_morpheme_analysis() {
        // lindera ko-dic splits agglutinative forms:
        //   "한국어로" → ["한국어", "로"]
        // So searching "한국어" should match content containing "한국어로"
        let index = Bm25Index::in_memory().unwrap();

        index
            .add("1", "한국어 설정", "사용자는 한국어로 대화를 선호합니다")
            .unwrap();
        index
            .add("2", "영어 설정", "영어로 코드 리뷰를 합니다")
            .unwrap();

        // "한국어" should match "한국어로" via morpheme splitting
        let results = index.search("한국어", 5).unwrap();
        assert!(!results.is_empty(), "Korean morpheme search should work");
        assert_eq!(results[0].0, "1");
    }
}
