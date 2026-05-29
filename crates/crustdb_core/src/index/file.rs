use crate::engine::RowId;
use crate::error::{CrustDbError, Result};
use crate::index::key::{decode_key, encode_key};
use crate::storage::record::{usize_to_u16, write_string, write_u16, write_u32, write_u64, Cursor};
use crate::value::Value;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const INDEX_MAGIC: &[u8; 8] = b"CRUSTBT1";
const INDEX_VERSION: u8 = 1;
const PAGE_SIZE: usize = 4096;
const PAGE_SIZE_U64: u64 = 4096;
const PAGE_KIND_LEAF: u8 = 1;
const PAGE_KIND_INTERNAL: u8 = 2;
const NO_PAGE: u64 = u64::MAX;

const PAGE_HEADER_LEN: usize = 5;
const LEAF_HEADER_LEN: usize = PAGE_HEADER_LEN + 8;
const INTERNAL_HEADER_LEN: usize = PAGE_HEADER_LEN + 8;

#[derive(Clone, Debug, PartialEq, Eq)]
struct EncodedEntry {
    key: Vec<u8>,
    row_id: RowId,
}

#[derive(Clone, Debug)]
struct PageRef {
    page_id: u64,
    first_key: Option<Vec<u8>>,
}

#[derive(Debug)]
struct BuiltIndex {
    pages: Vec<Vec<u8>>,
    root_page_id: u64,
}

#[derive(Debug)]
struct Header {
    root_page_id: u64,
    page_count: u64,
    entry_count: u64,
}

#[derive(Debug)]
struct LeafPage {
    next_leaf: Option<u64>,
    entries: Vec<EncodedEntry>,
}

#[derive(Debug)]
struct InternalPage {
    first_child: u64,
    separators: Vec<(Vec<u8>, u64)>,
}

pub(crate) fn index_path(root: &Path, model_name: &str, field_name: &str) -> PathBuf {
    root.join("indexes")
        .join(model_name)
        .join(format!("{field_name}.idx"))
}

pub(crate) fn save_index(
    root: &Path,
    model_name: &str,
    field_name: &str,
    entries: &[(Value, RowId)],
) -> Result<()> {
    let path = index_path(root, model_name, field_name);
    fs::create_dir_all(path.parent().expect("index file has parent"))?;

    let encoded_entries = sorted_encoded_entries(entries)?;
    let built = build_index_pages(&encoded_entries)?;
    let header = encode_header_page(
        model_name,
        field_name,
        built.root_page_id,
        built.pages.len() as u64,
        encoded_entries.len() as u64,
    )?;

    let mut file = File::create(path)?;
    file.write_all(&header)?;
    for page in built.pages {
        file.write_all(&page)?;
    }
    file.flush()?;
    Ok(())
}

pub(crate) fn load_index(
    root: &Path,
    model_name: &str,
    field_name: &str,
) -> Result<Vec<(Value, RowId)>> {
    let mut file = open_index_file(root, model_name, field_name)?;
    let header = read_header(&mut file, model_name, field_name)?;
    let mut page_id = leftmost_leaf_page_id(&mut file, &header)?;
    let mut entries = Vec::with_capacity(u64_to_usize(header.entry_count, "index entry count")?);
    let mut previous_key: Option<Vec<u8>> = None;
    let mut visited_pages = 0;

    loop {
        visited_pages += 1;
        if visited_pages > header.page_count {
            return Err(CrustDbError::StorageFormat(
                "index leaf chain contains a cycle".to_string(),
            ));
        }

        let page = read_page(&mut file, &header, page_id)?;
        let leaf = parse_leaf_page(&page)?;
        for entry in leaf.entries {
            if previous_key
                .as_ref()
                .is_some_and(|previous| previous >= &entry.key)
            {
                return Err(CrustDbError::StorageFormat(
                    "index keys are not strictly sorted".to_string(),
                ));
            }
            previous_key = Some(entry.key.clone());
            entries.push((decode_key(&entry.key)?, entry.row_id));
        }

        let Some(next_leaf) = leaf.next_leaf else {
            break;
        };
        page_id = next_leaf;
    }

    if entries.len() as u64 != header.entry_count {
        return Err(CrustDbError::StorageFormat(format!(
            "index entry count mismatch: expected {}, got {}",
            header.entry_count,
            entries.len()
        )));
    }

    Ok(entries)
}

pub(crate) fn lookup_index(
    root: &Path,
    model_name: &str,
    field_name: &str,
    value: &Value,
) -> Result<Option<RowId>> {
    let mut file = open_index_file(root, model_name, field_name)?;
    let header = read_header(&mut file, model_name, field_name)?;
    if header.entry_count == 0 {
        return Ok(None);
    }

    let target_key = encode_key(value);
    let mut page_id = header.root_page_id;
    let mut depth = 0;

    loop {
        depth += 1;
        if depth > header.page_count {
            return Err(CrustDbError::StorageFormat(
                "index tree traversal contains a cycle".to_string(),
            ));
        }

        let page = read_page(&mut file, &header, page_id)?;
        match page_kind(&page)? {
            PAGE_KIND_LEAF => {
                let leaf = parse_leaf_page(&page)?;
                return Ok(find_in_leaf(&leaf, &target_key));
            }
            PAGE_KIND_INTERNAL => {
                let internal = parse_internal_page(&page)?;
                page_id = child_for_key(&internal, &target_key);
            }
            other => {
                return Err(CrustDbError::StorageFormat(format!(
                    "unknown index page kind: {other}"
                )));
            }
        }
    }
}

fn sorted_encoded_entries(entries: &[(Value, RowId)]) -> Result<Vec<EncodedEntry>> {
    let mut encoded_entries = entries
        .iter()
        .map(|(value, row_id)| EncodedEntry {
            key: encode_key(value),
            row_id: *row_id,
        })
        .collect::<Vec<_>>();
    encoded_entries.sort_by(|left, right| left.key.cmp(&right.key));

    for pair in encoded_entries.windows(2) {
        if pair[0].key == pair[1].key {
            return Err(CrustDbError::StorageFormat(
                "duplicate key in index entries".to_string(),
            ));
        }
    }

    Ok(encoded_entries)
}

fn build_index_pages(entries: &[EncodedEntry]) -> Result<BuiltIndex> {
    let (mut pages, mut current_level) = build_leaf_level(entries)?;

    while current_level.len() > 1 {
        let groups = group_internal_children(&current_level)?;
        let mut next_level = Vec::with_capacity(groups.len());

        for children in groups {
            let page_id = pages.len() as u64;
            let page = encode_internal_page(&children)?;
            next_level.push(PageRef {
                page_id,
                first_key: children[0].first_key.clone(),
            });
            pages.push(page);
        }

        current_level = next_level;
    }

    Ok(BuiltIndex {
        pages,
        root_page_id: current_level[0].page_id,
    })
}

fn build_leaf_level(entries: &[EncodedEntry]) -> Result<(Vec<Vec<u8>>, Vec<PageRef>)> {
    if entries.is_empty() {
        return Ok((
            vec![encode_leaf_page(&[], None)?],
            vec![PageRef {
                page_id: 0,
                first_key: None,
            }],
        ));
    }

    let mut groups: Vec<Vec<EncodedEntry>> = Vec::new();
    let mut current = Vec::new();
    let mut current_len = LEAF_HEADER_LEN;

    for entry in entries {
        let entry_len = leaf_entry_len(&entry.key)?;
        if LEAF_HEADER_LEN + entry_len > PAGE_SIZE {
            return Err(CrustDbError::StorageFormat(
                "index key is too large for a leaf page".to_string(),
            ));
        }

        if !current.is_empty() && current_len + entry_len > PAGE_SIZE {
            groups.push(current);
            current = Vec::new();
            current_len = LEAF_HEADER_LEN;
        }

        current_len += entry_len;
        current.push(entry.clone());
    }

    if !current.is_empty() {
        groups.push(current);
    }

    let mut pages = Vec::with_capacity(groups.len());
    let mut refs = Vec::with_capacity(groups.len());

    for (index, group) in groups.iter().enumerate() {
        let page_id = index as u64;
        let next_leaf = if index + 1 < groups.len() {
            Some(page_id + 1)
        } else {
            None
        };
        pages.push(encode_leaf_page(group, next_leaf)?);
        refs.push(PageRef {
            page_id,
            first_key: Some(group[0].key.clone()),
        });
    }

    Ok((pages, refs))
}

fn group_internal_children(children: &[PageRef]) -> Result<Vec<Vec<PageRef>>> {
    let mut groups = Vec::new();
    let mut current = Vec::new();
    let mut current_len = INTERNAL_HEADER_LEN;

    for child in children {
        let child_len = if current.is_empty() {
            0
        } else {
            let Some(first_key) = child.first_key.as_ref() else {
                return Err(CrustDbError::StorageFormat(
                    "internal index child is missing a separator key".to_string(),
                ));
            };
            internal_separator_len(first_key)?
        };

        if !current.is_empty() && current_len + child_len > PAGE_SIZE {
            groups.push(current);
            current = Vec::new();
            current_len = INTERNAL_HEADER_LEN;
        }

        if current.is_empty() {
            current.push(child.clone());
        } else {
            current_len += child_len;
            current.push(child.clone());
        }
    }

    if !current.is_empty() {
        groups.push(current);
    }

    Ok(groups)
}

fn encode_header_page(
    model_name: &str,
    field_name: &str,
    root_page_id: u64,
    page_count: u64,
    entry_count: u64,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(INDEX_MAGIC);
    out.push(INDEX_VERSION);
    write_u32(&mut out, PAGE_SIZE as u32);
    write_u64(&mut out, root_page_id);
    write_u64(&mut out, page_count);
    write_u64(&mut out, entry_count);
    write_string(&mut out, model_name)?;
    write_string(&mut out, field_name)?;

    if out.len() > PAGE_SIZE {
        return Err(CrustDbError::StorageFormat(
            "index header is too large for a page".to_string(),
        ));
    }

    out.resize(PAGE_SIZE, 0);
    Ok(out)
}

fn encode_leaf_page(entries: &[EncodedEntry], next_leaf: Option<u64>) -> Result<Vec<u8>> {
    let mut out = page_header(PAGE_KIND_LEAF, entries.len())?;
    write_u64(&mut out, next_leaf.unwrap_or(NO_PAGE));

    for entry in entries {
        write_u16(&mut out, usize_to_u16(entry.key.len(), "index key length")?);
        out.extend_from_slice(&entry.key);
        write_u64(&mut out, entry.row_id);
    }

    finish_page(out)
}

fn encode_internal_page(children: &[PageRef]) -> Result<Vec<u8>> {
    if children.is_empty() {
        return Err(CrustDbError::StorageFormat(
            "internal index page must have at least one child".to_string(),
        ));
    }

    let mut out = page_header(PAGE_KIND_INTERNAL, children.len())?;
    write_u64(&mut out, children[0].page_id);

    for child in &children[1..] {
        let Some(first_key) = child.first_key.as_ref() else {
            return Err(CrustDbError::StorageFormat(
                "internal index child is missing a separator key".to_string(),
            ));
        };
        write_u16(
            &mut out,
            usize_to_u16(first_key.len(), "index separator key length")?,
        );
        out.extend_from_slice(first_key);
        write_u64(&mut out, child.page_id);
    }

    finish_page(out)
}

fn page_header(kind: u8, count: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.push(kind);
    write_u16(&mut out, usize_to_u16(count, "index page entry count")?);
    write_u16(&mut out, 0);
    Ok(out)
}

fn finish_page(mut out: Vec<u8>) -> Result<Vec<u8>> {
    if out.len() > PAGE_SIZE {
        return Err(CrustDbError::StorageFormat(
            "index page is larger than the configured page size".to_string(),
        ));
    }

    let used_len = usize_to_u16(out.len(), "index page used length")?;
    out[3..5].copy_from_slice(&used_len.to_le_bytes());
    out.resize(PAGE_SIZE, 0);
    Ok(out)
}

fn open_index_file(root: &Path, model_name: &str, field_name: &str) -> Result<File> {
    let path = index_path(root, model_name, field_name);
    if !path.exists() {
        return Err(CrustDbError::StorageFormat(format!(
            "missing index file: {model_name}.{field_name}"
        )));
    }
    Ok(File::open(path)?)
}

fn read_header(file: &mut File, model_name: &str, field_name: &str) -> Result<Header> {
    let actual_len = file.metadata()?.len();
    if actual_len < PAGE_SIZE_U64 {
        return Err(CrustDbError::StorageFormat(
            "index file is smaller than the header page".to_string(),
        ));
    }

    let mut page = vec![0; PAGE_SIZE];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut page)?;

    let mut cursor = Cursor::new(&page);
    for expected in INDEX_MAGIC {
        let found = cursor.read_u8()?;
        if found != *expected {
            return Err(CrustDbError::StorageFormat(
                "invalid index file magic".to_string(),
            ));
        }
    }

    let version = cursor.read_u8()?;
    if version != INDEX_VERSION {
        return Err(CrustDbError::StorageFormat(format!(
            "unsupported index file version: {version}"
        )));
    }

    let page_size = cursor.read_u32()? as usize;
    if page_size != PAGE_SIZE {
        return Err(CrustDbError::StorageFormat(format!(
            "unsupported index page size: {page_size}"
        )));
    }

    let root_page_id = cursor.read_u64()?;
    let page_count = cursor.read_u64()?;
    let entry_count = cursor.read_u64()?;
    let stored_model_name = cursor.read_string()?;
    let stored_field_name = cursor.read_string()?;
    if stored_model_name != model_name || stored_field_name != field_name {
        return Err(CrustDbError::StorageFormat(format!(
            "index file identity mismatch: expected {model_name}.{field_name}, got {stored_model_name}.{stored_field_name}"
        )));
    }

    if page_count == 0 {
        return Err(CrustDbError::StorageFormat(
            "index file has no pages".to_string(),
        ));
    }
    if root_page_id >= page_count {
        return Err(CrustDbError::StorageFormat(format!(
            "index root page is out of range: {root_page_id}"
        )));
    }

    let expected_len = PAGE_SIZE_U64
        .checked_add(page_count.checked_mul(PAGE_SIZE_U64).ok_or_else(|| {
            CrustDbError::StorageFormat("index file page count overflow".to_string())
        })?)
        .ok_or_else(|| CrustDbError::StorageFormat("index file length overflow".to_string()))?;
    if actual_len != expected_len {
        return Err(CrustDbError::StorageFormat(format!(
            "index file length mismatch: expected {expected_len}, got {actual_len}"
        )));
    }

    Ok(Header {
        root_page_id,
        page_count,
        entry_count,
    })
}

fn read_page(file: &mut File, header: &Header, page_id: u64) -> Result<Vec<u8>> {
    if page_id >= header.page_count {
        return Err(CrustDbError::StorageFormat(format!(
            "index page id is out of range: {page_id}"
        )));
    }

    let offset =
        PAGE_SIZE_U64
            .checked_add(page_id.checked_mul(PAGE_SIZE_U64).ok_or_else(|| {
                CrustDbError::StorageFormat("index page offset overflow".to_string())
            })?)
            .ok_or_else(|| CrustDbError::StorageFormat("index page offset overflow".to_string()))?;
    let mut page = vec![0; PAGE_SIZE];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut page)?;
    Ok(page)
}

fn page_kind(page: &[u8]) -> Result<u8> {
    let Some(kind) = page.first().copied() else {
        return Err(CrustDbError::StorageFormat("empty index page".to_string()));
    };
    Ok(kind)
}

fn parse_page_header(page: &[u8], expected_kind: u8) -> Result<(u16, usize)> {
    let mut cursor = Cursor::new(page);
    let kind = cursor.read_u8()?;
    if kind != expected_kind {
        return Err(CrustDbError::StorageFormat(format!(
            "unexpected index page kind: expected {expected_kind}, got {kind}"
        )));
    }

    let count = cursor.read_u16()?;
    let used_len = cursor.read_u16()? as usize;
    if !(PAGE_HEADER_LEN..=PAGE_SIZE).contains(&used_len) {
        return Err(CrustDbError::StorageFormat(format!(
            "invalid index page used length: {used_len}"
        )));
    }

    Ok((count, used_len))
}

fn parse_leaf_page(page: &[u8]) -> Result<LeafPage> {
    let (count, used_len) = parse_page_header(page, PAGE_KIND_LEAF)?;
    let mut cursor = Cursor::new(&page[..used_len]);
    let _kind = cursor.read_u8()?;
    let _count = cursor.read_u16()?;
    let _used_len = cursor.read_u16()?;
    let next_leaf = match cursor.read_u64()? {
        NO_PAGE => None,
        page_id => Some(page_id),
    };

    let mut entries = Vec::with_capacity(count as usize);
    let mut previous_key: Option<Vec<u8>> = None;
    for _ in 0..count {
        let key_len = cursor.read_u16()? as usize;
        let key = cursor.read_exact(key_len)?.to_vec();
        if previous_key
            .as_ref()
            .is_some_and(|previous| previous >= &key)
        {
            return Err(CrustDbError::StorageFormat(
                "leaf index keys are not strictly sorted".to_string(),
            ));
        }
        previous_key = Some(key.clone());
        let row_id = cursor.read_u64()?;
        entries.push(EncodedEntry { key, row_id });
    }

    if !cursor.is_finished() {
        return Err(CrustDbError::StorageFormat(
            "leaf index page has trailing bytes".to_string(),
        ));
    }

    Ok(LeafPage { next_leaf, entries })
}

fn parse_internal_page(page: &[u8]) -> Result<InternalPage> {
    let (child_count, used_len) = parse_page_header(page, PAGE_KIND_INTERNAL)?;
    if child_count == 0 {
        return Err(CrustDbError::StorageFormat(
            "internal index page has no children".to_string(),
        ));
    }

    let mut cursor = Cursor::new(&page[..used_len]);
    let _kind = cursor.read_u8()?;
    let _count = cursor.read_u16()?;
    let _used_len = cursor.read_u16()?;
    let first_child = cursor.read_u64()?;
    let mut separators = Vec::with_capacity(child_count.saturating_sub(1) as usize);
    let mut previous_key: Option<Vec<u8>> = None;

    for _ in 1..child_count {
        let key_len = cursor.read_u16()? as usize;
        let key = cursor.read_exact(key_len)?.to_vec();
        if previous_key
            .as_ref()
            .is_some_and(|previous| previous >= &key)
        {
            return Err(CrustDbError::StorageFormat(
                "internal index keys are not strictly sorted".to_string(),
            ));
        }
        previous_key = Some(key.clone());
        let child_page_id = cursor.read_u64()?;
        separators.push((key, child_page_id));
    }

    if !cursor.is_finished() {
        return Err(CrustDbError::StorageFormat(
            "internal index page has trailing bytes".to_string(),
        ));
    }

    Ok(InternalPage {
        first_child,
        separators,
    })
}

fn leftmost_leaf_page_id(file: &mut File, header: &Header) -> Result<u64> {
    let mut page_id = header.root_page_id;
    let mut depth = 0;

    loop {
        depth += 1;
        if depth > header.page_count {
            return Err(CrustDbError::StorageFormat(
                "index leftmost traversal contains a cycle".to_string(),
            ));
        }

        let page = read_page(file, header, page_id)?;
        match page_kind(&page)? {
            PAGE_KIND_LEAF => return Ok(page_id),
            PAGE_KIND_INTERNAL => {
                page_id = parse_internal_page(&page)?.first_child;
            }
            other => {
                return Err(CrustDbError::StorageFormat(format!(
                    "unknown index page kind: {other}"
                )));
            }
        }
    }
}

fn find_in_leaf(leaf: &LeafPage, target_key: &[u8]) -> Option<RowId> {
    leaf.entries
        .binary_search_by(|entry| entry.key.as_slice().cmp(target_key))
        .ok()
        .map(|index| leaf.entries[index].row_id)
}

fn child_for_key(internal: &InternalPage, target_key: &[u8]) -> u64 {
    let mut child_page_id = internal.first_child;
    for (separator, right_child_page_id) in &internal.separators {
        if target_key < separator {
            break;
        }
        child_page_id = *right_child_page_id;
    }
    child_page_id
}

fn leaf_entry_len(key: &[u8]) -> Result<usize> {
    usize_to_u16(key.len(), "index key length")?;
    Ok(2 + key.len() + 8)
}

fn internal_separator_len(key: &[u8]) -> Result<usize> {
    usize_to_u16(key.len(), "index separator key length")?;
    let len = 2 + key.len() + 8;
    if INTERNAL_HEADER_LEN + len > PAGE_SIZE {
        return Err(CrustDbError::StorageFormat(
            "index separator key is too large for an internal page".to_string(),
        ));
    }
    Ok(len)
}

fn u64_to_usize(value: u64, name: &str) -> Result<usize> {
    usize::try_from(value)
        .map_err(|_| CrustDbError::StorageFormat(format!("{name} is too large for this platform")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("crustdb-{label}-{}-{unique}", std::process::id()))
    }

    #[test]
    fn index_file_roundtrips_sorted_entries() {
        let root = temp_path("index-file");
        let entries = vec![
            (Value::String("bob".to_string()), 2),
            (Value::String("alice".to_string()), 1),
        ];

        save_index(&root, "User", "username", &entries).unwrap();
        let loaded = load_index(&root, "User", "username").unwrap();

        assert_eq!(
            loaded,
            vec![
                (Value::String("alice".to_string()), 1),
                (Value::String("bob".to_string()), 2)
            ]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn btree_index_lookup_traverses_multiple_pages() {
        let root = temp_path("index-btree-lookup");
        let entries = (0..500)
            .rev()
            .map(|value| (Value::Int(value), value as RowId + 10))
            .collect::<Vec<_>>();

        save_index(&root, "User", "id", &entries).unwrap();

        assert_eq!(
            lookup_index(&root, "User", "id", &Value::Int(0)).unwrap(),
            Some(10)
        );
        assert_eq!(
            lookup_index(&root, "User", "id", &Value::Int(249)).unwrap(),
            Some(259)
        );
        assert_eq!(
            lookup_index(&root, "User", "id", &Value::Int(499)).unwrap(),
            Some(509)
        );
        assert_eq!(
            lookup_index(&root, "User", "id", &Value::Int(500)).unwrap(),
            None
        );

        let mut file = open_index_file(&root, "User", "id").unwrap();
        let header = read_header(&mut file, "User", "id").unwrap();
        assert!(header.page_count > 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn empty_index_roundtrips_and_lookup_returns_none() {
        let root = temp_path("index-empty");

        save_index(&root, "User", "username", &[]).unwrap();

        assert_eq!(load_index(&root, "User", "username").unwrap(), vec![]);
        assert_eq!(
            lookup_index(
                &root,
                "User",
                "username",
                &Value::String("alice".to_string())
            )
            .unwrap(),
            None
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_index_file_returns_storage_format_error() {
        let root = temp_path("index-corrupt");
        let path = index_path(&root, "User", "id");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"not-an-index").unwrap();

        let err = load_index(&root, "User", "id").unwrap_err();

        assert!(err.to_string().contains("Storage format error"));

        let _ = fs::remove_dir_all(root);
    }
}
