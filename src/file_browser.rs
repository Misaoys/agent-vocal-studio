use anyhow::{ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

const PROJECT_EXT: [&str; 6] = ["avproj", "avpack", "mid", "midi", "ppsf", "json"];
const PAGE_SIZE: usize = 300;

pub(crate) fn display(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(network) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{network}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    }
}

#[derive(Serialize)]
struct Entry {
    path: String,
    name: String,
    directory: bool,
    bytes: Option<u64>,
}

pub(crate) fn browse(home: &Path, args: &Value) -> Result<Value> {
    let wanted = args["directory"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| home.to_path_buf());
    ensure!(wanted.is_absolute(), "请输入绝对目录路径");
    let directory = wanted.canonicalize().context("找不到目录")?;
    let extension = args["extension"].as_str();
    ensure!(
        extension.is_none_or(|ext| matches!(ext, "audio" | "zip")
            || PROJECT_EXT.contains(&ext)
            || crate::audio::IMPORT_EXTENSIONS.contains(&ext)),
        "不支持的文件类型"
    );
    let query = args["query"].as_str().unwrap_or("").trim().to_lowercase();
    let page = args["page"].as_u64().unwrap_or(0).min(100_000) as usize;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&directory).context("无法读取目录")?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !query.is_empty() && !name.to_lowercase().contains(&query) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let matched_file = metadata.is_file()
            && path
                .extension()
                .and_then(|part| part.to_str())
                .is_some_and(|ext| match extension {
                    Some("audio") => crate::audio::IMPORT_EXTENSIONS
                        .iter()
                        .any(|format| ext.eq_ignore_ascii_case(format)),
                    Some(wanted) => ext.eq_ignore_ascii_case(wanted),
                    None => {
                        ext.eq_ignore_ascii_case("zip")
                            || PROJECT_EXT
                                .iter()
                                .chain(&crate::audio::IMPORT_EXTENSIONS)
                                .any(|format| ext.eq_ignore_ascii_case(format))
                    }
                });
        if metadata.is_dir() || matched_file {
            entries.push(Entry {
                path: display(&path),
                name,
                directory: metadata.is_dir(),
                bytes: matched_file.then_some(metadata.len()),
            });
        }
    }
    entries.sort_by_key(|entry| (!entry.directory, entry.name.to_lowercase()));
    let total = entries.len();
    let mut locations = vec![json!({"label":"工程目录","path":display(home)})];
    if let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        for (label, name) in [("文档", "Documents"), ("桌面", "Desktop")] {
            let path = profile.join(name);
            if path.is_dir() {
                locations.push(json!({"label":label,"path":display(&path)}));
            }
        }
    }
    Ok(
        json!({"directory":display(&directory),"parent":directory.parent().map(display),"entries":entries.into_iter().skip(page*PAGE_SIZE).take(PAGE_SIZE).collect::<Vec<_>>(),"page":page,"pages":total.div_ceil(PAGE_SIZE),"locations":locations}),
    )
}

pub(crate) fn target(args: &Value, filename: &Path) -> Result<PathBuf> {
    let directory = Path::new(crate::model::field(args, "directory")?);
    let name = crate::model::field(args, "filename")?;
    ensure!(directory.is_absolute(), "请选择保存目录");
    ensure!(
        !name.is_empty()
            && name.chars().count() <= 220
            && !name.ends_with([' ', '.'])
            && !name
                .chars()
                .any(|character| character.is_control() || "/\\:*?\"<>|".contains(character)),
        "无效的文件名"
    );
    ensure!(
        Path::new(name).extension().is_some_and(|ext| filename
            .extension()
            .is_some_and(|source| ext.eq_ignore_ascii_case(source))),
        "扩展名应与导出格式相同"
    );
    let directory = directory.canonicalize().context("找不到保存目录")?;
    ensure!(directory.is_dir(), "请选择保存目录");
    let path = directory.join(name);
    ensure!(!path.is_dir(), "同名位置是目录");
    Ok(path)
}
