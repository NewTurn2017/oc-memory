use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

use crate::error::Result;
use crate::models::{Memory, MemoryMetadata, MemoryType, Priority};

/// SQLite-based metadata storage for memories
pub struct Storage {
    conn: Connection,
}

impl Storage {
    /// Open or create the database at the given path
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        let storage = Self { conn };
        storage.initialize()?;
        Ok(storage)
    }

    /// In-memory database (for testing)
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let storage = Self { conn };
        storage.initialize()?;
        Ok(storage)
    }

    fn initialize(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                title TEXT NOT NULL,
                memory_type TEXT NOT NULL,
                priority TEXT NOT NULL,
                source TEXT,
                tags TEXT NOT NULL DEFAULT '[]',
                concepts TEXT NOT NULL DEFAULT '[]',
                files TEXT NOT NULL DEFAULT '[]',
                embedding BLOB,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                accessed_at TEXT NOT NULL,
                access_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type);
            CREATE INDEX IF NOT EXISTS idx_memories_priority ON memories(priority);
            CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at);
            CREATE INDEX IF NOT EXISTS idx_memories_accessed ON memories(accessed_at);
            ",
        )?;
        Ok(())
    }

    /// Insert a new memory
    pub fn insert(&self, memory: &Memory) -> Result<()> {
        self.conn.execute(
            "INSERT INTO memories (id, content, title, memory_type, priority, source, tags, concepts, files, embedding, created_at, updated_at, accessed_at, access_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                memory.id,
                memory.content,
                memory.title,
                memory.metadata.memory_type.as_str(),
                serde_json::to_string(&memory.metadata.priority)?,
                memory.metadata.source,
                serde_json::to_string(&memory.metadata.tags)?,
                serde_json::to_string(&memory.metadata.concepts)?,
                serde_json::to_string(&memory.metadata.files)?,
                memory.embedding.as_ref().map(|v| {
                    v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()
                }),
                memory.created_at.to_rfc3339(),
                memory.updated_at.to_rfc3339(),
                memory.accessed_at.to_rfc3339(),
                memory.access_count,
            ],
        )?;
        Ok(())
    }

    /// Get a memory by ID
    pub fn get(&self, id: &str) -> Result<Option<Memory>> {
        let result = self
            .conn
            .query_row(
                "SELECT id, content, title, memory_type, priority, source, tags, concepts, files, embedding, created_at, updated_at, accessed_at, access_count
                 FROM memories WHERE id = ?1",
                params![id],
                |row| Ok(row_to_memory(row)),
            )
            .optional()?;

        match result {
            Some(Ok(memory)) => Ok(Some(memory)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    /// Get multiple memories by IDs
    pub fn get_many(&self, ids: &[String]) -> Result<Vec<Memory>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT id, content, title, memory_type, priority, source, tags, concepts, files, embedding, created_at, updated_at, accessed_at, access_count
             FROM memories WHERE id IN ({})",
            placeholders.join(", ")
        );

        let params: Vec<&dyn rusqlite::types::ToSql> = ids
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params.as_slice(), |row| Ok(row_to_memory(row)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        rows.into_iter().collect::<Result<Vec<_>>>()
    }

    /// Get all memory IDs and embeddings (for building vector index)
    pub fn all_embeddings(&self) -> Result<Vec<(String, Vec<f32>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, embedding FROM memories WHERE embedding IS NOT NULL")?;

        let rows = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let blob: Vec<u8> = row.get(1)?;
                let embedding: Vec<f32> = blob
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                    .collect();
                Ok((id, embedding))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Update access timestamp and count
    pub fn touch(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE memories SET accessed_at = ?1, access_count = access_count + 1 WHERE id = ?2",
            params![chrono::Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    /// Delete a memory by ID
    pub fn delete(&self, id: &str) -> Result<bool> {
        let affected = self
            .conn
            .execute("DELETE FROM memories WHERE id = ?1", params![id])?;
        Ok(affected > 0)
    }

    /// Total number of memories
    pub fn count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Get all (id, title, content) tuples for BM25 index rebuilding.
    /// This is lighter than loading full Memory objects.
    pub fn all_text_data(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, title, content FROM memories")?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(rows)
    }
}

fn row_to_memory(row: &rusqlite::Row<'_>) -> crate::error::Result<Memory> {
    let memory_type_str: String = row.get(3).map_err(crate::error::Error::Storage)?;
    let priority_str: String = row.get(4).map_err(crate::error::Error::Storage)?;
    let tags_str: String = row.get(6).map_err(crate::error::Error::Storage)?;
    let concepts_str: String = row.get(7).map_err(crate::error::Error::Storage)?;
    let files_str: String = row.get(8).map_err(crate::error::Error::Storage)?;
    let embedding_blob: Option<Vec<u8>> = row.get(9).map_err(crate::error::Error::Storage)?;

    let memory_type: MemoryType = serde_json::from_str(&format!("\"{memory_type_str}\""))?;
    let priority: Priority = serde_json::from_str(&priority_str)?;
    let tags: Vec<String> = serde_json::from_str(&tags_str)?;
    let concepts: Vec<String> = serde_json::from_str(&concepts_str)?;
    let files: Vec<String> = serde_json::from_str(&files_str)?;

    let embedding = embedding_blob.map(|blob| {
        blob.chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect::<Vec<f32>>()
    });

    let created_at_str: String = row.get(10).map_err(crate::error::Error::Storage)?;
    let updated_at_str: String = row.get(11).map_err(crate::error::Error::Storage)?;
    let accessed_at_str: String = row.get(12).map_err(crate::error::Error::Storage)?;

    Ok(Memory {
        id: row.get(0).map_err(crate::error::Error::Storage)?,
        content: row.get(1).map_err(crate::error::Error::Storage)?,
        title: row.get(2).map_err(crate::error::Error::Storage)?,
        metadata: MemoryMetadata {
            memory_type,
            priority,
            source: row.get(5).map_err(crate::error::Error::Storage)?,
            tags,
            concepts,
            files,
        },
        embedding,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .unwrap_or_default()
            .with_timezone(&chrono::Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .unwrap_or_default()
            .with_timezone(&chrono::Utc),
        accessed_at: chrono::DateTime::parse_from_rfc3339(&accessed_at_str)
            .unwrap_or_default()
            .with_timezone(&chrono::Utc),
        access_count: row
            .get::<_, i64>(13)
            .map_err(crate::error::Error::Storage)? as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MemoryMetadata;

    fn make(title: &str, content: &str) -> Memory {
        Memory::new(
            content.to_string(),
            title.to_string(),
            MemoryMetadata::default(),
        )
    }

    fn make_with_embedding(title: &str, content: &str, emb: Vec<f32>) -> Memory {
        let mut m = make(title, content);
        m.embedding = Some(emb);
        m
    }

    // ---- Basic CRUD ----

    #[test]
    fn test_insert_and_get() {
        let storage = Storage::in_memory().unwrap();
        let memory = Memory::new(
            "Rust는 메모리 안전한 시스템 프로그래밍 언어입니다".to_string(),
            "Rust 언어 특징".to_string(),
            MemoryMetadata {
                memory_type: MemoryType::Fact,
                priority: Priority::Medium,
                ..Default::default()
            },
        );
        storage.insert(&memory).unwrap();
        let retrieved = storage.get(&memory.id).unwrap().unwrap();
        assert_eq!(retrieved.content, memory.content);
        assert_eq!(retrieved.title, memory.title);
    }

    #[test]
    fn test_count() {
        let storage = Storage::in_memory().unwrap();
        assert_eq!(storage.count().unwrap(), 0);
        let memory = make("테스트", "테스트 메모리");
        storage.insert(&memory).unwrap();
        assert_eq!(storage.count().unwrap(), 1);
    }

    #[test]
    fn test_delete() {
        let storage = Storage::in_memory().unwrap();
        let memory = make("삭제", "삭제 테스트");
        storage.insert(&memory).unwrap();
        assert!(storage.delete(&memory.id).unwrap());
        assert!(storage.get(&memory.id).unwrap().is_none());
    }

    // ---- Production robustness ----

    #[test]
    fn test_delete_nonexistent_returns_false() {
        let storage = Storage::in_memory().unwrap();
        assert!(!storage.delete("nonexistent-id").unwrap());
    }

    #[test]
    fn test_get_nonexistent_returns_none() {
        let storage = Storage::in_memory().unwrap();
        assert!(storage.get("nonexistent-id").unwrap().is_none());
    }

    #[test]
    fn test_unicode_content_roundtrip() {
        let storage = Storage::in_memory().unwrap();
        let content = "한국어 🇰🇷 日本語 中文 العربية émojis: 🎉🔥💻 special: \"quotes\" & <tags>";
        let m = make("유니코드 테스트", content);
        storage.insert(&m).unwrap();
        let retrieved = storage.get(&m.id).unwrap().unwrap();
        assert_eq!(retrieved.content, content);
    }

    #[test]
    fn test_large_content() {
        let storage = Storage::in_memory().unwrap();
        // 100KB of Korean text
        let content: String = "가나다라마바사아자차카타파하".repeat(7500);
        assert!(content.len() > 100_000);
        let m = make("대용량", &content);
        storage.insert(&m).unwrap();
        let retrieved = storage.get(&m.id).unwrap().unwrap();
        assert_eq!(retrieved.content, content);
    }

    #[test]
    fn test_empty_strings() {
        let storage = Storage::in_memory().unwrap();
        let m = make("", "");
        storage.insert(&m).unwrap();
        let retrieved = storage.get(&m.id).unwrap().unwrap();
        assert_eq!(retrieved.content, "");
        assert_eq!(retrieved.title, "");
    }

    #[test]
    fn test_embedding_roundtrip_precision() {
        let storage = Storage::in_memory().unwrap();
        let emb = vec![
            0.123456789_f32,
            -0.987654321,
            0.0,
            f32::MIN_POSITIVE,
            1.0,
            -1.0,
        ];
        let m = make_with_embedding("임베딩", "정밀도 테스트", emb.clone());
        storage.insert(&m).unwrap();
        let retrieved = storage.get(&m.id).unwrap().unwrap();
        let ret_emb = retrieved.embedding.unwrap();
        assert_eq!(ret_emb.len(), emb.len());
        for (a, b) in emb.iter().zip(ret_emb.iter()) {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "Float bit-exact roundtrip failed: {} vs {}",
                a,
                b
            );
        }
    }

    #[test]
    fn test_null_embedding_roundtrip() {
        let storage = Storage::in_memory().unwrap();
        let m = make("임베딩 없음", "임베딩 없는 메모리");
        storage.insert(&m).unwrap();
        let retrieved = storage.get(&m.id).unwrap().unwrap();
        assert!(retrieved.embedding.is_none());
    }

    #[test]
    fn test_all_embeddings() {
        let storage = Storage::in_memory().unwrap();
        let m1 = make_with_embedding("A", "a", vec![1.0, 2.0]);
        let m2 = make("B", "b"); // no embedding
        let m3 = make_with_embedding("C", "c", vec![3.0, 4.0]);
        storage.insert(&m1).unwrap();
        storage.insert(&m2).unwrap();
        storage.insert(&m3).unwrap();
        let embeddings = storage.all_embeddings().unwrap();
        assert_eq!(
            embeddings.len(),
            2,
            "Should only return memories with embeddings"
        );
    }

    #[test]
    fn test_all_text_data() {
        let storage = Storage::in_memory().unwrap();
        storage.insert(&make("제목1", "내용1")).unwrap();
        storage.insert(&make("제목2", "내용2")).unwrap();
        storage.insert(&make("제목3", "내용3")).unwrap();
        let texts = storage.all_text_data().unwrap();
        assert_eq!(texts.len(), 3);
        for (_, title, content) in &texts {
            assert!(!title.is_empty());
            assert!(!content.is_empty());
        }
    }

    #[test]
    fn test_touch_updates_access() {
        let storage = Storage::in_memory().unwrap();
        let m = make("터치", "접근 추적");
        storage.insert(&m).unwrap();

        let before = storage.get(&m.id).unwrap().unwrap();
        assert_eq!(before.access_count, 0);

        storage.touch(&m.id).unwrap();
        storage.touch(&m.id).unwrap();
        storage.touch(&m.id).unwrap();

        let after = storage.get(&m.id).unwrap().unwrap();
        assert_eq!(after.access_count, 3);
        assert!(after.accessed_at >= before.accessed_at);
    }

    #[test]
    fn test_get_many() {
        let storage = Storage::in_memory().unwrap();
        let m1 = make("A", "aaa");
        let m2 = make("B", "bbb");
        let m3 = make("C", "ccc");
        storage.insert(&m1).unwrap();
        storage.insert(&m2).unwrap();
        storage.insert(&m3).unwrap();

        let results = storage.get_many(&[m1.id.clone(), m3.id.clone()]).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_get_many_empty() {
        let storage = Storage::in_memory().unwrap();
        let results = storage.get_many(&[]).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_many_with_invalid_ids() {
        let storage = Storage::in_memory().unwrap();
        let m1 = make("A", "aaa");
        storage.insert(&m1).unwrap();
        let results = storage
            .get_many(&[m1.id.clone(), "invalid-id".to_string()])
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_metadata_roundtrip() {
        let storage = Storage::in_memory().unwrap();
        let m = Memory::new(
            "내용".to_string(),
            "제목".to_string(),
            MemoryMetadata {
                memory_type: MemoryType::Decision,
                priority: Priority::High,
                source: Some("test-source".to_string()),
                tags: vec!["rust".to_string(), "한국어".to_string()],
                concepts: vec!["concept1".to_string()],
                files: vec!["src/main.rs".to_string()],
            },
        );
        storage.insert(&m).unwrap();
        let retrieved = storage.get(&m.id).unwrap().unwrap();
        assert_eq!(retrieved.metadata.memory_type, MemoryType::Decision);
        assert_eq!(retrieved.metadata.priority, Priority::High);
        assert_eq!(retrieved.metadata.source.as_deref(), Some("test-source"));
        assert_eq!(retrieved.metadata.tags, vec!["rust", "한국어"]);
        assert_eq!(retrieved.metadata.concepts, vec!["concept1"]);
        assert_eq!(retrieved.metadata.files, vec!["src/main.rs"]);
    }

    #[test]
    fn test_all_memory_types() {
        let storage = Storage::in_memory().unwrap();
        let types = [
            MemoryType::Observation,
            MemoryType::Decision,
            MemoryType::Preference,
            MemoryType::Fact,
            MemoryType::Task,
            MemoryType::Session,
            MemoryType::Bugfix,
            MemoryType::Discovery,
        ];
        for mt in &types {
            let m = Memory::new(
                format!("content for {:?}", mt),
                format!("title for {:?}", mt),
                MemoryMetadata {
                    memory_type: *mt,
                    ..Default::default()
                },
            );
            storage.insert(&m).unwrap();
            let r = storage.get(&m.id).unwrap().unwrap();
            assert_eq!(r.metadata.memory_type, *mt);
        }
        assert_eq!(storage.count().unwrap(), 8);
    }

    #[test]
    fn test_all_priority_levels() {
        let storage = Storage::in_memory().unwrap();
        for p in &[Priority::Low, Priority::Medium, Priority::High] {
            let m = Memory::new(
                "content".to_string(),
                format!("{:?}", p),
                MemoryMetadata {
                    priority: *p,
                    ..Default::default()
                },
            );
            storage.insert(&m).unwrap();
            let r = storage.get(&m.id).unwrap().unwrap();
            assert_eq!(r.metadata.priority, *p);
        }
    }

    #[test]
    fn test_bulk_insert_100() {
        let storage = Storage::in_memory().unwrap();
        for i in 0..100 {
            let m = make(&format!("메모리 {}", i), &format!("내용 {}", i));
            storage.insert(&m).unwrap();
        }
        assert_eq!(storage.count().unwrap(), 100);
        let texts = storage.all_text_data().unwrap();
        assert_eq!(texts.len(), 100);
    }

    #[test]
    fn test_disk_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let id;
        // Write
        {
            let storage = Storage::open(&db_path).unwrap();
            let m = make("영구 저장", "디스크 테스트");
            id = m.id.clone();
            storage.insert(&m).unwrap();
        }
        // Reopen and read
        {
            let storage = Storage::open(&db_path).unwrap();
            let retrieved = storage.get(&id).unwrap().unwrap();
            assert_eq!(retrieved.title, "영구 저장");
            assert_eq!(retrieved.content, "디스크 테스트");
        }
    }
}
