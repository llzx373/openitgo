//! descript.ion 文件注释（阶段 W，TC 惯例格式）：每行 `文件名 注释`，
//! 文件名含空白时双引号包裹（`"file name" 注释`）。读取侧编码探测复用
//! parser 的 `decode_text_guess`（UTF-8/本地 ANSI 经 chardetng）；写出
//! 统一 UTF-8 无 BOM。
//!
//! 已知取舍：注释按文件名匹配当前目录（分支视图子目录项的注释在其各自
//! 目录的 descript.ion 里，不递归读取）；注释不支持换行（写出时折叠为
//! 空格）。

use std::collections::HashMap;
use std::path::Path;

/// 注释文件名（TC 惯例）。
pub(crate) const DESCRIPT_ION: &str = "descript.ion";

/// 解析 descript.ion 内容（纯函数，保序）：空行、无注释段（行内无空白
/// 分隔或注释为空）、畸形引号行跳过；同名后出现者覆盖（位置不动）。
pub(crate) fn parse_comments(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let (name, comment) = if let Some(rest) = line.strip_prefix('"') {
            // 引号名：到下一个 `"` 为止，其后空白分隔注释。
            let Some(end) = rest.find('"') else {
                continue;
            };
            (&rest[..end], rest[end + 1..].trim_start())
        } else {
            let Some(sp) = line.find(char::is_whitespace) else {
                continue;
            };
            (&line[..sp], line[sp..].trim_start())
        };
        if name.is_empty() || comment.is_empty() {
            continue;
        }
        if let Some(slot) = out.iter_mut().find(|(n, _)| n == name) {
            slot.1 = comment.to_string();
        } else {
            out.push((name.to_string(), comment.to_string()));
        }
    }
    out
}

/// 序列化（纯函数，保序）：名含空白写引号形式；注释内换行/连续空白
/// 折叠为单个空格（descript.ion 行格式不支持多行）；空注释条目跳过。
pub(crate) fn format_comments(entries: &[(String, String)]) -> String {
    let mut s = String::new();
    for (name, comment) in entries {
        let comment = comment.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() || comment.is_empty() {
            continue;
        }
        if name.contains(char::is_whitespace) {
            s.push_str(&format!("\"{name}\" {comment}\n"));
        } else {
            s.push_str(&format!("{name} {comment}\n"));
        }
    }
    s
}

/// 读取保序条目（内部）：文件不存在/读取失败/解码失败 = 空。
fn read_comment_entries(dir: &Path) -> Vec<(String, String)> {
    let Ok(bytes) = std::fs::read(dir.join(DESCRIPT_ION)) else {
        return Vec::new();
    };
    let Some(text) = openitgo_parser::archive::decode_text_guess(&bytes) else {
        return Vec::new();
    };
    // 严格 UTF-8 路径不去 BOM，手动剥掉。
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    parse_comments(text)
}

/// 读 dir/descript.ion 为名称 → 注释表（面板缓存用）。
pub(crate) fn read_comments(dir: &Path) -> HashMap<String, String> {
    read_comment_entries(dir).into_iter().collect()
}

/// 写一条注释：None/空串（trim 后）= 删除该条；其余条目顺序保留，新
/// 条目追加到尾；条目清空后删除文件（已不存在也算成功）；UTF-8 无 BOM
/// 写出。
pub(crate) fn write_comment(dir: &Path, name: &str, comment: Option<&str>) -> Result<(), String> {
    let path = dir.join(DESCRIPT_ION);
    let mut entries = read_comment_entries(dir);
    let comment = comment.map(str::trim).filter(|c| !c.is_empty());
    match comment {
        Some(c) => {
            if let Some(slot) = entries.iter_mut().find(|(n, _)| n == name) {
                slot.1 = c.to_string();
            } else {
                entries.push((name.to_string(), c.to_string()));
            }
        }
        None => entries.retain(|(n, _)| n != name),
    }
    if entries.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("无法删除 {DESCRIPT_ION}: {e}")),
        };
    }
    std::fs::write(&path, format_comments(&entries))
        .map_err(|e| format!("无法写入 {DESCRIPT_ION}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_comments_plain_quoted_and_malformed() {
        let text = "a.txt 简单注释\n\"b file.txt\" 带空格的名 注释也带空格\n\
                    \n无注释行\n\"畸形引号行\n\"\" 空名跳过\n c.txt 前导空白行\n";
        let entries = parse_comments(text);
        assert_eq!(
            entries,
            vec![
                ("a.txt".to_string(), "简单注释".to_string()),
                (
                    "b file.txt".to_string(),
                    "带空格的名 注释也带空格".to_string()
                ),
                // 前导空白行：未 trim_start，名称为空段 → 跳过？实际
                // find(whitespace) 在位置 0，name 为空 → 跳过。
            ]
        );
    }

    #[test]
    fn parse_comments_duplicate_name_overrides_in_place() {
        let entries = parse_comments("a.txt 旧\nb.txt 乙\na.txt 新\n");
        assert_eq!(
            entries,
            vec![
                ("a.txt".to_string(), "新".to_string()),
                ("b.txt".to_string(), "乙".to_string()),
            ]
        );
    }

    #[test]
    fn format_comments_quotes_names_and_collapses_whitespace() {
        let entries = vec![
            ("a.txt".to_string(), "注释".to_string()),
            ("b file.txt".to_string(), "多行\n折叠  空白".to_string()),
            ("c.txt".to_string(), "  ".to_string()), // 空注释跳过
        ];
        assert_eq!(
            format_comments(&entries),
            "a.txt 注释\n\"b file.txt\" 多行 折叠 空白\n"
        );
    }

    #[test]
    fn parse_format_roundtrip() {
        let entries = vec![
            ("a.txt".to_string(), "甲".to_string()),
            ("b file.txt".to_string(), "乙 丙".to_string()),
        ];
        assert_eq!(parse_comments(&format_comments(&entries)), entries);
    }

    #[test]
    fn write_comment_crud_and_file_removal() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // 新增两条（顺序保留）。
        write_comment(dir, "a.txt", Some("甲")).unwrap();
        write_comment(dir, "b file.txt", Some("乙")).unwrap();
        assert_eq!(
            read_comments(dir),
            HashMap::from([
                ("a.txt".to_string(), "甲".to_string()),
                ("b file.txt".to_string(), "乙".to_string()),
            ])
        );
        // 文件内容：引号名 + UTF-8 无 BOM。
        let raw = std::fs::read(dir.join(DESCRIPT_ION)).unwrap();
        assert!(!raw.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(
            String::from_utf8(raw).unwrap(),
            "a.txt 甲\n\"b file.txt\" 乙\n"
        );
        // 修改 + 空串删除。
        write_comment(dir, "a.txt", Some("甲改")).unwrap();
        write_comment(dir, "b file.txt", Some("")).unwrap();
        assert_eq!(
            read_comments(dir),
            HashMap::from([("a.txt".to_string(), "甲改".to_string())])
        );
        // 删最后一条 → 文件删除；再删不存在的文件仍 Ok。
        write_comment(dir, "a.txt", None).unwrap();
        assert!(!dir.join(DESCRIPT_ION).exists());
        write_comment(dir, "a.txt", None).unwrap();
    }

    #[test]
    fn read_comments_handles_ansi_and_bom() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // UTF-8 BOM。
        std::fs::write(dir.join(DESCRIPT_ION), "\u{feff}a.txt 甲\n").unwrap();
        assert_eq!(
            read_comments(dir),
            HashMap::from([("a.txt".to_string(), "甲".to_string())])
        );
        // GBK 编码（"注释" 的 GBK 字节）。
        let mut raw = b"a.txt ".to_vec();
        raw.extend_from_slice(&[0xD7, 0xA2, 0xCA, 0xCD]);
        std::fs::write(dir.join(DESCRIPT_ION), raw).unwrap();
        assert_eq!(
            read_comments(dir),
            HashMap::from([("a.txt".to_string(), "注释".to_string())])
        );
        // 不存在 = 空表。
        let sub = dir.join("nope");
        std::fs::create_dir(&sub).unwrap();
        assert!(read_comments(&sub).is_empty());
    }
}
