//! Unit tests for the `apply_patch` module.
//!
//! Moved out of `apply_patch.rs` into its own module so the implementation
//! file stays focused on the patching logic.

#[cfg(test)]
mod tests {
    use crate::engine::apply_patch::*;
    use std::path::PathBuf;

    fn temp_workspace() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "anacleto_apply_patch_test_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_batch_object() {
        let json = r#"{"operations":[{"op":"add","path":"a.txt","content":"hi"}]}"#;
        let batch = parse_patch_batch(json).unwrap();
        assert_eq!(batch.operations.len(), 1);
        assert_eq!(batch.operations[0].op, PatchOpKind::Add);
        assert_eq!(batch.operations[0].path, "a.txt");
        assert_eq!(batch.operations[0].content.as_deref(), Some("hi"));
    }

    #[test]
    fn parse_batch_bare_array() {
        let json = r#"[{"op":"delete","path":"b.txt"}]"#;
        let batch = parse_patch_batch(json).unwrap();
        assert_eq!(batch.operations[0].op, PatchOpKind::Delete);
    }

    #[test]
    fn parse_batch_invalid_json() {
        assert!(parse_patch_batch("not json").is_err());
    }

    #[test]
    fn parse_batch_empty() {
        assert!(parse_patch_batch(r#"{"operations":[]}"#).is_err());
    }

    #[test]
    fn apply_add_update_delete() {
        let ws = temp_workspace();

        // Add
        let add = parse_patch_batch(
            r#"{"operations":[{"op":"add","path":"dir/nested/f.txt","content":"hello"}]}"#,
        )
        .unwrap();
        let results = apply_patch_batch(&ws, &add, false).unwrap();
        assert_eq!(results, vec!["Added dir/nested/f.txt".to_string()]);
        let file = ws.join("dir/nested/f.txt");
        assert!(file.exists());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello");

        // Update
        let upd = parse_patch_batch(
            r#"{"operations":[{"op":"update","path":"dir/nested/f.txt","content":"world"}]}"#,
        )
        .unwrap();
        let results = apply_patch_batch(&ws, &upd, false).unwrap();
        assert_eq!(results, vec!["Updated dir/nested/f.txt".to_string()]);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "world");

        // Delete
        let del =
            parse_patch_batch(r#"{"operations":[{"op":"delete","path":"dir/nested/f.txt"}]}"#)
                .unwrap();
        let results = apply_patch_batch(&ws, &del, false).unwrap();
        assert_eq!(results, vec!["Deleted dir/nested/f.txt".to_string()]);
        assert!(!file.exists());

        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn apply_add_creates_parent_dirs() {
        let ws = temp_workspace();
        let batch =
            parse_patch_batch(r#"{"operations":[{"op":"add","path":"a/b/c.txt","content":"x"}]}"#)
                .unwrap();
        apply_patch_batch(&ws, &batch, false).unwrap();
        assert!(ws.join("a/b/c.txt").exists());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn apply_update_preserves_crlf() {
        let ws = temp_workspace();
        let file = ws.join("crlf.txt");
        std::fs::write(&file, b"line1\r\nline2\r\n").unwrap();

        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"update","path":"crlf.txt","content":"new1\nnew2\n"}]}"#,
        )
        .unwrap();
        apply_patch_batch(&ws, &batch, false).unwrap();

        let bytes = std::fs::read(&file).unwrap();
        assert_eq!(bytes, b"new1\r\nnew2\r\n");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn apply_update_preserves_bom() {
        let ws = temp_workspace();
        let file = ws.join("bom.txt");
        let mut original = vec![0xEF, 0xBB, 0xBF];
        original.extend_from_slice(b"hello");
        std::fs::write(&file, &original).unwrap();

        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"update","path":"bom.txt","content":"world"}]}"#,
        )
        .unwrap();
        apply_patch_batch(&ws, &batch, false).unwrap();

        let bytes = std::fs::read(&file).unwrap();
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(&bytes[3..], b"world");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn apply_update_preserves_bom_and_crlf() {
        let ws = temp_workspace();
        let file = ws.join("both.txt");
        let mut original = vec![0xEF, 0xBB, 0xBF];
        original.extend_from_slice(b"a\r\nb\r\n");
        std::fs::write(&file, &original).unwrap();

        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"update","path":"both.txt","content":"x\ny\n"}]}"#,
        )
        .unwrap();
        apply_patch_batch(&ws, &batch, false).unwrap();

        let bytes = std::fs::read(&file).unwrap();
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(&bytes[3..], b"x\r\ny\r\n");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn apply_add_uses_default_encoding() {
        let ws = temp_workspace();
        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"add","path":"new.txt","content":"a\nb\n"}]}"#,
        )
        .unwrap();
        apply_patch_batch(&ws, &batch, false).unwrap();
        let bytes = std::fs::read(ws.join("new.txt")).unwrap();
        assert!(!bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(bytes, b"a\nb\n");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn reject_path_traversal() {
        let ws = temp_workspace();
        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"add","path":"../escape.txt","content":"x"}]}"#,
        )
        .unwrap();
        let err = apply_patch_batch(&ws, &batch, false).unwrap_err();
        assert!(err.contains("escapes workspace"));
        // Nothing should have been created.
        assert!(!ws.join("../escape.txt").exists());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn reject_absolute_path() {
        let ws = temp_workspace();
        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"add","path":"/etc/passwd","content":"x"}]}"#,
        )
        .unwrap();
        assert!(apply_patch_batch(&ws, &batch, false).is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn external_path_requires_allow_external() {
        let ws = temp_workspace();
        let outside = std::env::temp_dir().join(format!(
            "anacleto_apply_patch_outside_{}",
            uuid::Uuid::new_v4()
        ));

        // Without allow_external, an absolute external path is rejected.
        let batch = parse_patch_batch(&format!(
            r#"{{"operations":[{{"op":"add","path":"{}","content":"x"}}]}}"#,
            outside.display()
        ))
        .unwrap();
        assert!(apply_patch_batch(&ws, &batch, false).is_err());

        // With allow_external, it is permitted.
        let results = apply_patch_batch(&ws, &batch, true).unwrap();
        assert_eq!(results.len(), 1);
        assert!(outside.exists());

        std::fs::remove_dir_all(&ws).unwrap();
        std::fs::remove_file(&outside).unwrap();
    }

    #[test]
    fn reject_nested_traversal() {
        let ws = temp_workspace();
        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"add","path":"a/../../escape.txt","content":"x"}]}"#,
        )
        .unwrap();
        assert!(apply_patch_batch(&ws, &batch, false).is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn reject_update_missing_file() {
        let ws = temp_workspace();
        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"update","path":"missing.txt","content":"x"}]}"#,
        )
        .unwrap();
        assert!(apply_patch_batch(&ws, &batch, false).is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn reject_add_existing_file() {
        let ws = temp_workspace();
        std::fs::write(ws.join("exists.txt"), "x").unwrap();
        let batch =
            parse_patch_batch(r#"{"operations":[{"op":"add","path":"exists.txt","content":"y"}]}"#)
                .unwrap();
        assert!(apply_patch_batch(&ws, &batch, false).is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn detect_encoding_helpers() {
        assert_eq!(
            detect_encoding(b"plain\n"),
            FileEncoding {
                has_bom: false,
                crlf: false
            }
        );
        assert_eq!(
            detect_encoding(b"a\r\nb"),
            FileEncoding {
                has_bom: false,
                crlf: true
            }
        );
        assert_eq!(
            detect_encoding(&[0xEF, 0xBB, 0xBF, b'a']),
            FileEncoding {
                has_bom: true,
                crlf: false
            }
        );
    }

    #[test]
    fn encode_content_does_not_double_crlf() {
        let enc = FileEncoding {
            has_bom: false,
            crlf: true,
        };
        // Content that already contains CRLF must not be doubled.
        let out = encode_content("a\r\nb\nc", &enc);
        assert_eq!(out, b"a\r\nb\r\nc");
    }
}
