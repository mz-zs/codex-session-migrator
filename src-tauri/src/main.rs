#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{DateTime, Local, Utc};
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{params_from_iter, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const PACKAGE_FORMAT: &str = "codex-session-migration";
const PACKAGE_VERSION: u32 = 1;

#[derive(Debug, Clone)]
struct RelatedTableSpec {
    name: &'static str,
    where_columns: &'static [&'static str],
}

const RELATED_TABLES: &[RelatedTableSpec] = &[
    RelatedTableSpec {
        name: "thread_goals",
        where_columns: &["thread_id"],
    },
    RelatedTableSpec {
        name: "thread_dynamic_tools",
        where_columns: &["thread_id"],
    },
    RelatedTableSpec {
        name: "thread_spawn_edges",
        where_columns: &["parent_thread_id", "child_thread_id"],
    },
    RelatedTableSpec {
        name: "stage1_outputs",
        where_columns: &["thread_id"],
    },
];

type JsonRow = Map<String, Value>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Diagnosis {
    sqlite_available: bool,
    sqlite_error: String,
    default_codex_home: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectListResponse {
    codex_home: String,
    projects: Vec<ProjectSummary>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProjectSummary {
    cwd: String,
    display_cwd: String,
    name: String,
    session_count: i64,
    updated_at_ms: Option<i64>,
    updated_text: String,
    session_ids: Option<Vec<String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionSummary {
    id: String,
    title: String,
    cwd: String,
    rollout_path: String,
    display_rollout_path: String,
    exists: bool,
    file_size: u64,
    updated_at_ms: Option<i64>,
    created_at_ms: Option<i64>,
    updated_text: String,
    created_text: String,
    first_user_message: String,
    archived: bool,
    model: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportOptions {
    codex_home: String,
    session_ids: Vec<String>,
    export_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportResult {
    export_path: String,
    session_count: usize,
    project_count: usize,
    size: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportOptions {
    codex_home: String,
    package_path: String,
    session_ids: Vec<String>,
    target_cwd: String,
    add_workspace_root: bool,
    overwrite_files: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportResult {
    imported_count: usize,
    backup_dir: String,
    codex_home: String,
    project_path: String,
    sessions: Vec<ImportedSession>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportedSession {
    id: String,
    title: String,
    cwd: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: String,
    format_version: u32,
    exported_at: String,
    source: ManifestSource,
    thread_columns: Vec<TableColumn>,
    related_rows: BTreeMap<String, Vec<JsonRow>>,
    projects: Vec<ProjectSummary>,
    sessions: Vec<ManifestSession>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestSource {
    codex_home: String,
    hostname: String,
    platform: String,
    arch: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ManifestSession {
    id: String,
    title: String,
    cwd: String,
    display_cwd: String,
    rollout_relative_path: String,
    archive_path: String,
    thread: JsonRow,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageSummary {
    package_path: String,
    exported_at: String,
    source: ManifestSource,
    projects: Vec<ProjectSummary>,
    sessions: Vec<PackageSessionSummary>,
    session_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageSessionSummary {
    id: String,
    title: String,
    cwd: String,
    display_cwd: String,
    updated_at_ms: Option<i64>,
    updated_text: String,
    first_user_message: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct TableColumn {
    cid: i64,
    name: String,
    #[serde(rename = "type")]
    column_type: String,
    notnull: i64,
    dflt_value: Option<String>,
    pk: i64,
}

fn main() {
    if let Ok(mode) = env::var("CODEX_MIGRATOR_SMOKE") {
        if let Err(error) = run_smoke(&mode) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            diagnose,
            select_codex_home,
            select_project_path,
            select_package,
            select_export_path,
            load_projects,
            load_sessions,
            export_sessions,
            inspect_package,
            import_sessions,
            open_path
        ])
        .run(tauri::generate_context!())
        .expect("failed to run tauri app");
}

fn run_smoke(mode: &str) -> Result<(), String> {
    let codex_home = path_to_string(default_codex_home());
    let projects = load_projects(codex_home.clone())?;
    if mode == "projects" {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "projectCount": projects.projects.len(),
                "firstProject": projects.projects.first()
            }))
            .map_err(to_string_error)?
        );
        return Ok(());
    }

    let project = projects
        .projects
        .first()
        .ok_or_else(|| "没有可测试的项目。".to_string())?;
    let sessions = load_sessions(codex_home.clone(), project.cwd.clone())?;
    let session = sessions
        .iter()
        .find(|session| session.exists)
        .ok_or_else(|| "没有可测试的会话文件。".to_string())?;
    let temp_dir = env::temp_dir().join(format!("codex-migrator-{}", timestamp_for_file_name()));
    fs::create_dir_all(&temp_dir).map_err(to_string_error)?;
    let export_path = temp_dir.join("smoke.codexpack");
    let exported = export_sessions(ExportOptions {
        codex_home: codex_home.clone(),
        session_ids: vec![session.id.clone()],
        export_path: path_to_string(&export_path),
    })?;
    let inspected = inspect_package(path_to_string(&export_path))?;
    let mut imported = Value::Null;

    if mode == "roundtrip" {
        let target_home = temp_dir.join("target-codex-home");
        fs::create_dir_all(&target_home).map_err(to_string_error)?;
        for name in [
            "state_5.sqlite",
            "state_5.sqlite-wal",
            "state_5.sqlite-shm",
            "session_index.jsonl",
            ".codex-global-state.json",
        ] {
            let source = PathBuf::from(&codex_home).join(name);
            if source.exists() {
                fs::copy(&source, target_home.join(name)).map_err(to_string_error)?;
            }
        }
        imported = serde_json::to_value(import_sessions(ImportOptions {
            codex_home: path_to_string(&target_home),
            package_path: path_to_string(&export_path),
            session_ids: vec![session.id.clone()],
            target_cwd: path_to_string(temp_dir.join("TargetProject")),
            add_workspace_root: true,
            overwrite_files: true,
        })?)
        .map_err(to_string_error)?;
    }

    println!(
        "{}",
        serde_json::to_string(&json!({
            "exported": exported,
            "inspectedSessionCount": inspected.session_count,
            "imported": imported
        }))
        .map_err(to_string_error)?
    );
    Ok(())
}

#[tauri::command]
fn diagnose() -> Diagnosis {
    Diagnosis {
        sqlite_available: true,
        sqlite_error: String::new(),
        default_codex_home: default_codex_home().to_string_lossy().to_string(),
    }
}

#[tauri::command]
fn select_codex_home(current_path: String) -> Option<String> {
    let mut dialog = rfd::FileDialog::new().set_title("选择 Codex 数据目录");
    if !current_path.trim().is_empty() {
        dialog = dialog.set_directory(strip_long_path_prefix(&current_path));
    } else {
        dialog = dialog.set_directory(default_codex_home());
    }
    dialog.pick_folder().map(path_to_string)
}

#[tauri::command]
fn select_project_path() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("选择目标项目目录")
        .pick_folder()
        .map(path_to_string)
}

#[tauri::command]
fn select_package() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("选择迁移包")
        .add_filter("Codex Session Package", &["codexpack", "zip"])
        .add_filter("All Files", &["*"])
        .pick_file()
        .map(path_to_string)
}

#[tauri::command]
fn select_export_path(default_name: String) -> Option<String> {
    let file_name = if default_name.trim().is_empty() {
        "codex-sessions.codexpack".to_string()
    } else {
        default_name
    };

    let desktop = dirs_fallback_desktop();
    rfd::FileDialog::new()
        .set_title("保存迁移包")
        .set_directory(desktop)
        .set_file_name(&file_name)
        .add_filter("Codex Session Package", &["codexpack"])
        .add_filter("Zip Archive", &["zip"])
        .save_file()
        .map(path_to_string)
}

#[tauri::command]
fn load_projects(codex_home: String) -> Result<ProjectListResponse, String> {
    let codex_home = ensure_codex_home(&codex_home)?;
    let conn = open_db(&codex_home)?;

    let mut stmt = conn
        .prepare(
            r#"
            SELECT
              cwd,
              COUNT(*) AS session_count,
              MAX(updated_at_ms) AS updated_at_ms,
              MAX(updated_at) AS updated_at
            FROM threads
            GROUP BY cwd
            ORDER BY updated_at_ms DESC, updated_at DESC
            "#,
        )
        .map_err(to_string_error)?;

    let rows = stmt
        .query_map([], |row| {
            let cwd: String = row.get(0)?;
            let session_count: i64 = row.get(1)?;
            let updated_at_ms: Option<i64> = row.get(2)?;
            let updated_at: Option<i64> = row.get(3)?;
            let final_ms = updated_at_ms.or_else(|| updated_at.map(|value| value * 1000));
            Ok(ProjectSummary {
                display_cwd: display_path(&cwd),
                name: project_name(&cwd),
                cwd,
                session_count,
                updated_at_ms: final_ms,
                updated_text: format_time(final_ms),
                session_ids: None,
            })
        })
        .map_err(to_string_error)?;

    let mut projects = Vec::new();
    for row in rows {
        projects.push(row.map_err(to_string_error)?);
    }

    Ok(ProjectListResponse {
        codex_home: path_to_string(codex_home),
        projects,
    })
}

#[tauri::command]
fn load_sessions(codex_home: String, cwd: String) -> Result<Vec<SessionSummary>, String> {
    let codex_home = ensure_codex_home(&codex_home)?;
    if cwd.trim().is_empty() {
        return Err("项目路径不能为空。".to_string());
    }

    let conn = open_db(&codex_home)?;
    let mut stmt = conn
        .prepare(
            r#"
            SELECT
              id,
              title,
              cwd,
              rollout_path,
              created_at,
              updated_at,
              created_at_ms,
              updated_at_ms,
              first_user_message,
              archived,
              model,
              model_provider
            FROM threads
            WHERE cwd = ?
            ORDER BY updated_at_ms DESC, updated_at DESC
            "#,
        )
        .map_err(to_string_error)?;

    let rows = stmt
        .query_map([cwd], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let row_cwd: String = row.get(2)?;
            let rollout_path: String = row.get(3)?;
            let created_at: Option<i64> = row.get(4)?;
            let updated_at: Option<i64> = row.get(5)?;
            let created_at_ms: Option<i64> = row.get(6)?;
            let updated_at_ms: Option<i64> = row.get(7)?;
            let first_user_message: Option<String> = row.get(8)?;
            let archived: Option<i64> = row.get(9)?;
            let model: Option<String> = row.get(10)?;
            let model_provider: Option<String> = row.get(11)?;

            Ok((
                id,
                title,
                row_cwd,
                rollout_path,
                created_at,
                updated_at,
                created_at_ms,
                updated_at_ms,
                first_user_message.unwrap_or_default(),
                archived.unwrap_or(0) != 0,
                model.or(model_provider).unwrap_or_default(),
            ))
        })
        .map_err(to_string_error)?;

    let mut sessions = Vec::new();
    for row in rows {
        let (
            id,
            title,
            row_cwd,
            rollout_path,
            created_at,
            updated_at,
            created_at_ms,
            updated_at_ms,
            first_user_message,
            archived,
            model,
        ) = row.map_err(to_string_error)?;
        let resolved = resolve_rollout_path(
            &json_row_from_pairs(&[("id", id.clone()), ("rollout_path", rollout_path.clone())]),
            &codex_home,
            false,
        )?;
        let exists = resolved.as_ref().map(|path| path.exists()).unwrap_or(false);
        let file_size = resolved
            .as_ref()
            .and_then(|path| fs::metadata(path).ok())
            .map(|meta| meta.len())
            .unwrap_or(0);
        let final_updated_ms = updated_at_ms.or_else(|| updated_at.map(|value| value * 1000));
        let final_created_ms = created_at_ms.or_else(|| created_at.map(|value| value * 1000));

        sessions.push(SessionSummary {
            id,
            title: if title.is_empty() { "(无标题)".to_string() } else { title },
            cwd: row_cwd,
            display_rollout_path: display_path(&rollout_path),
            rollout_path,
            exists,
            file_size,
            updated_at_ms: final_updated_ms,
            created_at_ms: final_created_ms,
            updated_text: format_time(final_updated_ms),
            created_text: format_time(final_created_ms),
            first_user_message,
            archived,
            model,
        });
    }

    Ok(sessions)
}

#[tauri::command]
fn export_sessions(options: ExportOptions) -> Result<ExportResult, String> {
    let codex_home = ensure_codex_home(&options.codex_home)?;
    let session_ids = unique_strings(&options.session_ids);
    if session_ids.is_empty() {
        return Err("至少选择一条会话。".to_string());
    }
    if options.export_path.trim().is_empty() {
        return Err("导出路径不能为空。".to_string());
    }

    let conn = open_db(&codex_home)?;
    let threads = get_threads_by_ids(&conn, &session_ids)?;
    if threads.is_empty() {
        return Err("没有找到选中的会话。".to_string());
    }

    let found: BTreeSet<String> = threads
        .iter()
        .filter_map(|row| json_string(row, "id"))
        .collect();
    let missing: Vec<String> = session_ids
        .iter()
        .filter(|id| !found.contains(*id))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(format!("这些会话没有找到：{}", missing.join(", ")));
    }

    let export_path = PathBuf::from(&options.export_path);
    if let Some(parent) = export_path.parent() {
        fs::create_dir_all(parent).map_err(to_string_error)?;
    }

    let file = File::create(&export_path).map_err(to_string_error)?;
    let mut zip = ZipWriter::new(file);
    let zip_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let related_rows = export_related_rows(&conn, &session_ids)?;
    let thread_columns = get_table_info(&conn, "threads")?;
    let mut sessions = Vec::new();

    for thread in &threads {
        let id = json_string(thread, "id").ok_or_else(|| "会话缺少 id。".to_string())?;
        let rollout_path = resolve_rollout_path(thread, &codex_home, true)?
            .ok_or_else(|| format!("找不到会话 {id} 的 jsonl 文件。"))?;
        let rollout_relative_path = get_rollout_relative_path(&rollout_path, &codex_home)?;
        let archive_path = format!("files/{}", to_zip_path(&rollout_relative_path));

        zip.start_file(&archive_path, zip_options)
            .map_err(to_string_error)?;
        let mut session_file = File::open(&rollout_path).map_err(to_string_error)?;
        std::io::copy(&mut session_file, &mut zip).map_err(to_string_error)?;

        let title = json_string(thread, "title").unwrap_or_default();
        let cwd = json_string(thread, "cwd").unwrap_or_default();
        sessions.push(ManifestSession {
            id,
            title,
            display_cwd: display_path(&cwd),
            cwd,
            rollout_relative_path: to_zip_path(&rollout_relative_path),
            archive_path,
            thread: thread.clone(),
        });
    }

    let projects = build_package_projects(&sessions);
    let manifest = Manifest {
        format: PACKAGE_FORMAT.to_string(),
        format_version: PACKAGE_VERSION,
        exported_at: Utc::now().to_rfc3339(),
        source: ManifestSource {
            codex_home: path_to_string(&codex_home),
            hostname: hostname_fallback(),
            platform: env::consts::OS.to_string(),
            arch: env::consts::ARCH.to_string(),
        },
        thread_columns,
        related_rows,
        projects,
        sessions,
    };

    zip.start_file("manifest.json", zip_options)
        .map_err(to_string_error)?;
    zip.write_all(
        serde_json::to_string_pretty(&manifest)
            .map_err(to_string_error)?
            .as_bytes(),
    )
    .map_err(to_string_error)?;
    zip.finish().map_err(to_string_error)?;

    let size = fs::metadata(&export_path).map_err(to_string_error)?.len();
    Ok(ExportResult {
        export_path: path_to_string(export_path),
        session_count: manifest.sessions.len(),
        project_count: manifest.projects.len(),
        size,
    })
}

#[tauri::command]
fn inspect_package(package_path: String) -> Result<PackageSummary, String> {
    let manifest = read_manifest(&package_path)?;
    Ok(summarize_manifest(package_path, manifest))
}

#[tauri::command]
fn import_sessions(options: ImportOptions) -> Result<ImportResult, String> {
    let codex_home = ensure_codex_home(&options.codex_home)?;
    let manifest = read_manifest(&options.package_path)?;
    let selected_ids = unique_strings(&options.session_ids);
    let sessions: Vec<ManifestSession> = if selected_ids.is_empty() {
        manifest.sessions.clone()
    } else {
        let selected: BTreeSet<String> = selected_ids.into_iter().collect();
        manifest
            .sessions
            .iter()
            .filter(|session| selected.contains(&session.id))
            .cloned()
            .collect()
    };

    if sessions.is_empty() {
        return Err("迁移包里没有可导入的会话。".to_string());
    }

    let target_cwd = if options.target_cwd.trim().is_empty() {
        String::new()
    } else {
        to_codex_cwd(&options.target_cwd)
    };
    let backup_dir = backup_codex_state(&codex_home, &options.package_path)?;

    let file = File::open(&options.package_path).map_err(to_string_error)?;
    let mut archive = ZipArchive::new(file).map_err(to_string_error)?;
    let mut conn = open_db(&codex_home)?;
    let tx = conn.transaction().map_err(to_string_error)?;
    let mut imported_details = Vec::new();
    let mut imported_threads = Vec::new();

    for session in &sessions {
        let target_rollout_path = write_rollout_from_package(
            &mut archive,
            session,
            &codex_home,
            options.overwrite_files,
        )?;
        let mut thread = session.thread.clone();
        thread.insert("id".to_string(), Value::String(session.id.clone()));
        thread.insert(
            "title".to_string(),
            Value::String(
                json_string(&thread, "title")
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| session.title.clone()),
            ),
        );
        thread.insert(
            "cwd".to_string(),
            Value::String(if target_cwd.is_empty() {
                json_string(&thread, "cwd")
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| session.cwd.clone())
            } else {
                target_cwd.clone()
            }),
        );
        thread.insert(
            "rollout_path".to_string(),
            Value::String(path_to_string(&target_rollout_path)),
        );

        upsert_row(&tx, "threads", &thread)?;
        imported_details.push(ImportedSession {
            id: session.id.clone(),
            title: json_string(&thread, "title").unwrap_or_else(|| session.id.clone()),
            cwd: json_string(&thread, "cwd").unwrap_or_default(),
        });
        imported_threads.push(thread);
    }

    let imported_ids: BTreeSet<String> = imported_details.iter().map(|item| item.id.clone()).collect();
    import_related_rows(&tx, &manifest.related_rows, &imported_ids)?;
    tx.commit().map_err(to_string_error)?;

    update_session_index(&codex_home, &imported_threads)?;
    if options.add_workspace_root && !target_cwd.is_empty() {
        update_global_state(&codex_home, &strip_long_path_prefix(&target_cwd))?;
    }

    Ok(ImportResult {
        imported_count: imported_details.len(),
        backup_dir: path_to_string(&backup_dir),
        codex_home: path_to_string(&codex_home),
        project_path: target_cwd,
        sessions: imported_details,
    })
}

#[tauri::command]
fn open_path(file_path: String) -> Result<(), String> {
    if file_path.trim().is_empty() {
        return Ok(());
    }
    opener::open(file_path).map_err(to_string_error)
}

fn default_codex_home() -> PathBuf {
    if let Ok(value) = env::var("CODEX_HOME") {
        if !value.trim().is_empty() {
            return PathBuf::from(value);
        }
    }
    home_dir().join(".codex")
}

fn home_dir() -> PathBuf {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn dirs_fallback_desktop() -> PathBuf {
    let desktop = home_dir().join("Desktop");
    if desktop.exists() {
        desktop
    } else {
        home_dir()
    }
}

fn ensure_codex_home(input: &str) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Codex 数据目录不能为空。".to_string());
    }
    let path = PathBuf::from(strip_long_path_prefix(input));
    if !path.exists() {
        return Err(format!("Codex 数据目录不存在：{}", path.display()));
    }
    let db_path = path.join("state_5.sqlite");
    if !db_path.exists() {
        return Err(format!(
            "没有找到 state_5.sqlite。请先在这台电脑打开过 Codex：{}",
            db_path.display()
        ));
    }
    Ok(path)
}

fn open_db(codex_home: &Path) -> Result<Connection, String> {
    let conn = Connection::open(codex_home.join("state_5.sqlite")).map_err(to_string_error)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(to_string_error)?;
    Ok(conn)
}

fn get_threads_by_ids(conn: &Connection, ids: &[String]) -> Result<Vec<JsonRow>, String> {
    let placeholders = placeholders(ids.len());
    let sql = format!("SELECT * FROM threads WHERE id IN ({placeholders})");
    let mut stmt = conn.prepare(&sql).map_err(to_string_error)?;
    query_json_rows(&mut stmt, ids.iter().map(|id| SqlValue::Text(id.clone())).collect())
}

fn export_related_rows(
    conn: &Connection,
    ids: &[String],
) -> Result<BTreeMap<String, Vec<JsonRow>>, String> {
    let table_names = get_table_names(conn)?;
    let mut output = BTreeMap::new();

    for spec in RELATED_TABLES {
        if !table_names.contains(spec.name) {
            continue;
        }
        let columns: BTreeSet<String> = get_table_info(conn, spec.name)?
            .into_iter()
            .map(|column| column.name)
            .collect();
        let usable: Vec<&str> = spec
            .where_columns
            .iter()
            .copied()
            .filter(|column| columns.contains(*column))
            .collect();
        if usable.is_empty() {
            continue;
        }

        let mut rows = Vec::new();
        for column in usable {
            let sql = format!(
                "SELECT * FROM {} WHERE {} IN ({})",
                quote_ident(spec.name),
                quote_ident(column),
                placeholders(ids.len())
            );
            let mut stmt = conn.prepare(&sql).map_err(to_string_error)?;
            rows.extend(query_json_rows(
                &mut stmt,
                ids.iter().map(|id| SqlValue::Text(id.clone())).collect(),
            )?);
        }
        output.insert(spec.name.to_string(), dedupe_rows(rows));
    }

    Ok(output)
}

fn import_related_rows(
    conn: &Connection,
    related_rows: &BTreeMap<String, Vec<JsonRow>>,
    imported_ids: &BTreeSet<String>,
) -> Result<(), String> {
    let table_names = get_table_names(conn)?;
    for (table_name, rows) in related_rows {
        if !table_names.contains(table_name.as_str()) {
            continue;
        }
        for row in rows {
            let thread_id = json_string(row, "thread_id")
                .or_else(|| json_string(row, "parent_thread_id"))
                .or_else(|| json_string(row, "child_thread_id"));
            if let Some(thread_id) = thread_id {
                if !imported_ids.contains(&thread_id) {
                    continue;
                }
            }
            upsert_row(conn, table_name, row)?;
        }
    }
    Ok(())
}

fn upsert_row(conn: &Connection, table_name: &str, source_row: &JsonRow) -> Result<(), String> {
    let table_info = get_table_info(conn, table_name)?;
    if table_info.is_empty() {
        return Err(format!("目标数据库没有表：{table_name}"));
    }

    let row = apply_required_defaults(&table_info, source_row);
    let columns: Vec<String> = table_info
        .iter()
        .map(|column| column.name.clone())
        .filter(|name| row.contains_key(name))
        .collect();
    if columns.is_empty() {
        return Ok(());
    }

    let values: Vec<SqlValue> = columns
        .iter()
        .map(|column| json_to_sql_value(row.get(column).unwrap_or(&Value::Null)))
        .collect::<Result<Vec<_>, _>>()?;
    let quoted_columns = columns
        .iter()
        .map(|column| quote_ident(column))
        .collect::<Vec<_>>()
        .join(",");

    let mut pk_columns: Vec<(i64, String)> = table_info
        .iter()
        .filter(|column| column.pk > 0)
        .map(|column| (column.pk, column.name.clone()))
        .collect();
    pk_columns.sort_by_key(|(pk, _)| *pk);
    let pk_names: Vec<String> = pk_columns.into_iter().map(|(_, name)| name).collect();

    if !pk_names.is_empty() {
        let update_columns: Vec<String> = columns
            .iter()
            .filter(|column| !pk_names.contains(column))
            .cloned()
            .collect();
        let conflict_target = pk_names
            .iter()
            .map(|column| quote_ident(column))
            .collect::<Vec<_>>()
            .join(",");
        let update_sql = if update_columns.is_empty() {
            " DO NOTHING".to_string()
        } else {
            format!(
                " DO UPDATE SET {}",
                update_columns
                    .iter()
                    .map(|column| format!(
                        "{} = excluded.{}",
                        quote_ident(column),
                        quote_ident(column)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT ({}){}",
            quote_ident(table_name),
            quoted_columns,
            placeholders(columns.len()),
            conflict_target,
            update_sql
        );
        conn.execute(&sql, params_from_iter(values))
            .map_err(to_string_error)?;
        return Ok(());
    }

    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        quote_ident(table_name),
        quoted_columns,
        placeholders(columns.len())
    );
    conn.execute(&sql, params_from_iter(values))
        .map_err(to_string_error)?;
    Ok(())
}

fn apply_required_defaults(table_info: &[TableColumn], source_row: &JsonRow) -> JsonRow {
    let mut row = source_row.clone();
    let now_seconds = Utc::now().timestamp();
    let now_ms = Utc::now().timestamp_millis();

    let mut defaults = BTreeMap::new();
    defaults.insert(
        "rollout_path".to_string(),
        Value::String(json_string(&row, "rollout_path").unwrap_or_default()),
    );
    defaults.insert("created_at".to_string(), json!(now_seconds));
    defaults.insert("updated_at".to_string(), json!(now_seconds));
    defaults.insert(
        "source".to_string(),
        Value::String(json_string(&row, "source").unwrap_or_else(|| "vscode".to_string())),
    );
    defaults.insert(
        "model_provider".to_string(),
        Value::String(json_string(&row, "model_provider").unwrap_or_default()),
    );
    defaults.insert(
        "cwd".to_string(),
        Value::String(json_string(&row, "cwd").unwrap_or_default()),
    );
    defaults.insert(
        "title".to_string(),
        Value::String(
            json_string(&row, "title")
                .or_else(|| json_string(&row, "thread_name"))
                .or_else(|| json_string(&row, "id"))
                .unwrap_or_default(),
        ),
    );
    defaults.insert(
        "sandbox_policy".to_string(),
        Value::String(
            json_string(&row, "sandbox_policy").unwrap_or_else(|| "danger-full-access".to_string()),
        ),
    );
    defaults.insert(
        "approval_mode".to_string(),
        Value::String(json_string(&row, "approval_mode").unwrap_or_else(|| "never".to_string())),
    );
    defaults.insert("tokens_used".to_string(), json!(0));
    defaults.insert("has_user_event".to_string(), json!(1));
    defaults.insert("archived".to_string(), json!(0));
    defaults.insert(
        "cli_version".to_string(),
        Value::String(json_string(&row, "cli_version").unwrap_or_default()),
    );
    defaults.insert(
        "first_user_message".to_string(),
        Value::String(json_string(&row, "first_user_message").unwrap_or_default()),
    );
    defaults.insert(
        "memory_mode".to_string(),
        Value::String(json_string(&row, "memory_mode").unwrap_or_else(|| "enabled".to_string())),
    );
    defaults.insert(
        "created_at_ms".to_string(),
        json!(json_i64(&row, "created_at_ms")
            .or_else(|| json_i64(&row, "created_at").map(|value| value * 1000))
            .unwrap_or(now_ms)),
    );
    defaults.insert(
        "updated_at_ms".to_string(),
        json!(json_i64(&row, "updated_at_ms")
            .or_else(|| json_i64(&row, "updated_at").map(|value| value * 1000))
            .unwrap_or(now_ms)),
    );

    for column in table_info {
        if row.contains_key(&column.name) {
            continue;
        }
        if let Some(default_value) = defaults.get(&column.name) {
            row.insert(column.name.clone(), default_value.clone());
        } else if column.notnull == 1 && column.dflt_value.is_none() && column.pk == 0 {
            row.insert(column.name.clone(), Value::String(String::new()));
        }
    }

    row
}

fn write_rollout_from_package(
    archive: &mut ZipArchive<File>,
    session: &ManifestSession,
    codex_home: &Path,
    overwrite_files: bool,
) -> Result<PathBuf, String> {
    let relative = ensure_safe_relative_path(&session.rollout_relative_path)?;
    let target_path = codex_home.join(relative);
    if target_path.exists() && !overwrite_files {
        return Ok(target_path);
    }

    let mut entry = archive
        .by_name(&session.archive_path)
        .map_err(|_| format!("迁移包缺少会话文件：{}", session.archive_path))?;
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(to_string_error)?;
    }
    let mut output = File::create(&target_path).map_err(to_string_error)?;
    std::io::copy(&mut entry, &mut output).map_err(to_string_error)?;
    Ok(target_path)
}

fn backup_codex_state(codex_home: &Path, package_path: &str) -> Result<PathBuf, String> {
    let backup_dir = codex_home
        .join("backups_state")
        .join(format!("session-migrator-{}", timestamp_for_file_name()));
    fs::create_dir_all(&backup_dir).map_err(to_string_error)?;

    for name in [
        "state_5.sqlite",
        "state_5.sqlite-wal",
        "state_5.sqlite-shm",
        "session_index.jsonl",
        ".codex-global-state.json",
    ] {
        let source = codex_home.join(name);
        if source.exists() {
            fs::copy(&source, backup_dir.join(name)).map_err(to_string_error)?;
        }
    }

    let package = PathBuf::from(package_path);
    if package.exists() {
        fs::copy(&package, backup_dir.join(package.file_name().unwrap_or_default()))
            .map_err(to_string_error)?;
    }

    Ok(backup_dir)
}

fn update_session_index(codex_home: &Path, imported_threads: &[JsonRow]) -> Result<(), String> {
    let index_path = codex_home.join("session_index.jsonl");
    let imported_ids: BTreeSet<String> = imported_threads
        .iter()
        .filter_map(|thread| json_string(thread, "id"))
        .collect();
    let mut records = Vec::new();

    if index_path.exists() {
        let data = fs::read_to_string(&index_path).map_err(to_string_error)?;
        for line in data.lines().filter(|line| !line.trim().is_empty()) {
            if let Ok(record) = serde_json::from_str::<Value>(line) {
                let id = record
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if !id.is_empty() && !imported_ids.contains(&id) {
                    records.push(record);
                }
            }
        }
    }

    for thread in imported_threads {
        let id = json_string(thread, "id").unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let title = json_string(thread, "title").unwrap_or_else(|| id.clone());
        let updated_ms = json_i64(thread, "updated_at_ms")
            .or_else(|| json_i64(thread, "updated_at").map(|value| value * 1000))
            .unwrap_or_else(|| Utc::now().timestamp_millis());
        let updated_at = DateTime::<Utc>::from_timestamp_millis(updated_ms)
            .unwrap_or_else(Utc::now)
            .to_rfc3339();
        records.push(json!({
            "id": id,
            "thread_name": title,
            "updated_at": updated_at
        }));
    }

    let mut output = String::new();
    for record in records {
        output.push_str(&serde_json::to_string(&record).map_err(to_string_error)?);
        output.push('\n');
    }
    fs::write(index_path, output).map_err(to_string_error)
}

fn update_global_state(codex_home: &Path, target_cwd: &str) -> Result<(), String> {
    let state_path = codex_home.join(".codex-global-state.json");
    if !state_path.exists() {
        return Ok(());
    }

    let data = fs::read_to_string(&state_path).map_err(to_string_error)?;
    let mut state: Value = match serde_json::from_str(&data) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };

    let Some(root) = state
        .get_mut("electron-persisted-atom-state")
        .and_then(Value::as_object_mut)
    else {
        return Ok(());
    };

    let display = strip_long_path_prefix(target_cwd);
    add_unique_path(root, "electron-saved-workspace-roots", &display, false);
    add_unique_path(root, "project-order", &display, true);

    let tmp_path = state_path.with_extension(format!("json.tmp-{}", Utc::now().timestamp_millis()));
    fs::write(
        &tmp_path,
        serde_json::to_string_pretty(&state).map_err(to_string_error)?,
    )
    .map_err(to_string_error)?;
    fs::rename(tmp_path, state_path).map_err(to_string_error)
}

fn add_unique_path(root: &mut Map<String, Value>, key: &str, item: &str, prepend: bool) {
    let mut values: Vec<Value> = root
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| {
            entry
                .as_str()
                .map(|value| normalize_path_key(value) != normalize_path_key(item))
                .unwrap_or(false)
        })
        .collect();

    if prepend {
        values.insert(0, Value::String(item.to_string()));
    } else {
        values.push(Value::String(item.to_string()));
    }

    root.insert(key.to_string(), Value::Array(values));
}

fn read_manifest(package_path: &str) -> Result<Manifest, String> {
    let file = File::open(package_path).map_err(to_string_error)?;
    let mut archive = ZipArchive::new(file).map_err(to_string_error)?;
    let mut entry = archive
        .by_name("manifest.json")
        .map_err(|_| "这不是有效的 Codex 会话迁移包：缺少 manifest.json。".to_string())?;
    let mut data = String::new();
    entry.read_to_string(&mut data).map_err(to_string_error)?;
    let manifest: Manifest = serde_json::from_str(&data).map_err(to_string_error)?;
    if manifest.format != PACKAGE_FORMAT {
        return Err("迁移包格式不匹配。".to_string());
    }
    Ok(manifest)
}

fn summarize_manifest(package_path: String, manifest: Manifest) -> PackageSummary {
    let sessions = manifest
        .sessions
        .iter()
        .map(|session| {
            let updated_at_ms = json_i64(&session.thread, "updated_at_ms")
                .or_else(|| json_i64(&session.thread, "updated_at").map(|value| value * 1000));
            let cwd = if session.cwd.is_empty() {
                json_string(&session.thread, "cwd").unwrap_or_default()
            } else {
                session.cwd.clone()
            };
            PackageSessionSummary {
                id: session.id.clone(),
                title: if session.title.is_empty() {
                    json_string(&session.thread, "title").unwrap_or_else(|| session.id.clone())
                } else {
                    session.title.clone()
                },
                display_cwd: display_path(&cwd),
                cwd,
                updated_at_ms,
                updated_text: format_time(updated_at_ms),
                first_user_message: json_string(&session.thread, "first_user_message").unwrap_or_default(),
            }
        })
        .collect::<Vec<_>>();

    let session_count = sessions.len();
    PackageSummary {
        package_path,
        exported_at: manifest.exported_at,
        source: manifest.source,
        projects: if manifest.projects.is_empty() {
            build_package_projects(&manifest.sessions)
        } else {
            manifest.projects
        },
        sessions,
        session_count,
    }
}

fn build_package_projects(sessions: &[ManifestSession]) -> Vec<ProjectSummary> {
    let mut map: BTreeMap<String, ProjectSummary> = BTreeMap::new();
    for session in sessions {
        let cwd = if session.cwd.is_empty() {
            json_string(&session.thread, "cwd").unwrap_or_default()
        } else {
            session.cwd.clone()
        };
        let key = normalize_path_key(if cwd.is_empty() { "(projectless)" } else { &cwd });
        let entry = map.entry(key).or_insert_with(|| ProjectSummary {
            display_cwd: display_path(&cwd),
            name: project_name(&cwd),
            cwd: cwd.clone(),
            session_count: 0,
            updated_at_ms: None,
            updated_text: String::new(),
            session_ids: Some(Vec::new()),
        });
        entry.session_count += 1;
        if let Some(ids) = entry.session_ids.as_mut() {
            ids.push(session.id.clone());
        }
    }
    map.into_values().collect()
}

fn get_table_names(conn: &Connection) -> Result<BTreeSet<String>, String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .map_err(to_string_error)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(to_string_error)?;
    let mut names = BTreeSet::new();
    for row in rows {
        names.insert(row.map_err(to_string_error)?);
    }
    Ok(names)
}

fn get_table_info(conn: &Connection, table_name: &str) -> Result<Vec<TableColumn>, String> {
    let sql = format!("PRAGMA table_info({})", quote_ident(table_name));
    let mut stmt = conn.prepare(&sql).map_err(to_string_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(TableColumn {
                cid: row.get(0)?,
                name: row.get(1)?,
                column_type: row.get(2)?,
                notnull: row.get(3)?,
                dflt_value: row.get(4)?,
                pk: row.get(5)?,
            })
        })
        .map_err(to_string_error)?;
    let mut columns = Vec::new();
    for row in rows {
        columns.push(row.map_err(to_string_error)?);
    }
    Ok(columns)
}

fn query_json_rows(
    stmt: &mut rusqlite::Statement,
    params: Vec<SqlValue>,
) -> Result<Vec<JsonRow>, String> {
    let column_names: Vec<String> = stmt.column_names().iter().map(|name| name.to_string()).collect();
    let rows = stmt
        .query_map(params_from_iter(params), |row| {
            let mut object = Map::new();
            for (idx, name) in column_names.iter().enumerate() {
                let value = match row.get_ref(idx)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(value) => json!(value),
                    ValueRef::Real(value) => json!(value),
                    ValueRef::Text(value) => Value::String(String::from_utf8_lossy(value).to_string()),
                    ValueRef::Blob(value) => json!({ "__blob": BASE64.encode(value) }),
                };
                object.insert(name.clone(), value);
            }
            Ok(object)
        })
        .map_err(to_string_error)?;

    let mut output = Vec::new();
    for row in rows {
        output.push(row.map_err(to_string_error)?);
    }
    Ok(output)
}

fn resolve_rollout_path(
    row: &JsonRow,
    codex_home: &Path,
    required: bool,
) -> Result<Option<PathBuf>, String> {
    let direct = json_string(row, "rollout_path")
        .map(|value| PathBuf::from(strip_long_path_prefix(&value)))
        .filter(|path| path.exists());
    if direct.is_some() {
        return Ok(direct);
    }

    let id = json_string(row, "id").unwrap_or_default();
    if let Some(found) = find_rollout_by_id(&codex_home.join("sessions"), &id) {
        return Ok(Some(found));
    }

    if required {
        let raw = json_string(row, "rollout_path").unwrap_or_else(|| "(empty)".to_string());
        return Err(format!("找不到会话 {id} 的 jsonl 文件：{raw}"));
    }

    Ok(json_string(row, "rollout_path").map(|value| PathBuf::from(strip_long_path_prefix(&value))))
}

fn find_rollout_by_id(root: &Path, id: &str) -> Option<PathBuf> {
    if id.is_empty() || !root.exists() {
        return None;
    }

    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file() {
            let name = entry.file_name().to_string_lossy();
            if name.contains(id) && name.ends_with(".jsonl") {
                return Some(entry.into_path());
            }
        }
    }
    None
}

fn get_rollout_relative_path(rollout_path: &Path, codex_home: &Path) -> Result<String, String> {
    let clean_rollout = PathBuf::from(strip_long_path_prefix(&path_to_string(rollout_path)));
    let clean_home = PathBuf::from(strip_long_path_prefix(&path_to_string(codex_home)));
    if let Ok(relative) = clean_rollout.strip_prefix(&clean_home) {
        return ensure_safe_relative_path(&path_to_string(relative));
    }

    let parts: Vec<String> = clean_rollout
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect();
    if let Some(index) = parts.iter().position(|part| part == "sessions") {
        return ensure_safe_relative_path(&parts[index..].join(std::path::MAIN_SEPARATOR_STR));
    }

    ensure_safe_relative_path(&format!(
        "sessions{}imported{}{}",
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR,
        clean_rollout
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "imported.jsonl".to_string())
    ))
}

fn ensure_safe_relative_path(relative_path: &str) -> Result<String, String> {
    let normalized = PathBuf::from(relative_path.replace(['\\', '/'], std::path::MAIN_SEPARATOR_STR));
    if normalized.as_os_str().is_empty()
        || normalized.is_absolute()
        || normalized.components().any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!("迁移包包含不安全路径：{relative_path}"));
    }
    Ok(path_to_string(normalized))
}

fn display_path(value: &str) -> String {
    strip_long_path_prefix(value)
}

fn strip_long_path_prefix(value: &str) -> String {
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        value.to_string()
    }
}

fn to_codex_cwd(value: &str) -> String {
    let clean = path_to_string(PathBuf::from(strip_long_path_prefix(value)));
    if cfg!(windows) && clean.len() >= 3 && clean.as_bytes()[1] == b':' {
        format!(r"\\?\{clean}")
    } else {
        clean
    }
}

fn normalize_path_key(value: &str) -> String {
    let mut clean = strip_long_path_prefix(value);
    while clean.ends_with('\\') || clean.ends_with('/') {
        clean.pop();
    }
    if cfg!(windows) {
        clean.to_lowercase()
    } else {
        clean
    }
}

fn project_name(cwd: &str) -> String {
    let clean = strip_long_path_prefix(cwd);
    if clean.is_empty() {
        return "(无项目)".to_string();
    }
    Path::new(&clean)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or(clean)
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(",")
}

fn json_to_sql_value(value: &Value) -> Result<SqlValue, String> {
    Ok(match value {
        Value::Null => SqlValue::Null,
        Value::Bool(value) => SqlValue::Integer(if *value { 1 } else { 0 }),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                SqlValue::Integer(value)
            } else if let Some(value) = number.as_f64() {
                SqlValue::Real(value)
            } else {
                SqlValue::Null
            }
        }
        Value::String(value) => SqlValue::Text(value.clone()),
        Value::Object(object) if object.contains_key("__blob") => {
            let encoded = object
                .get("__blob")
                .and_then(Value::as_str)
                .ok_or_else(|| "blob 字段格式错误。".to_string())?;
            SqlValue::Blob(BASE64.decode(encoded).map_err(to_string_error)?)
        }
        other => SqlValue::Text(other.to_string()),
    })
}

fn json_string(row: &JsonRow, key: &str) -> Option<String> {
    match row.get(key) {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        Some(Value::Bool(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn json_i64(row: &JsonRow, key: &str) -> Option<i64> {
    match row.get(key) {
        Some(Value::Number(value)) => value.as_i64(),
        Some(Value::String(value)) => value.parse().ok(),
        _ => None,
    }
}

fn json_row_from_pairs(pairs: &[(&str, String)]) -> JsonRow {
    let mut row = Map::new();
    for (key, value) in pairs {
        row.insert((*key).to_string(), Value::String(value.clone()));
    }
    row
}

fn format_time(ms: Option<i64>) -> String {
    let Some(ms) = ms else {
        return String::new();
    };
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|datetime| {
            datetime
                .with_timezone(&Local)
                .format("%Y/%m/%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn timestamp_for_file_name() -> String {
    Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn to_zip_path(value: &str) -> String {
    value.replace('\\', "/")
}

fn unique_strings(values: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
            output.push(trimmed.to_string());
        }
    }
    output
}

fn dedupe_rows(rows: Vec<JsonRow>) -> Vec<JsonRow> {
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for row in rows {
        if let Ok(key) = serde_json::to_string(&row) {
            if seen.insert(key) {
                output.push(row);
            }
        }
    }
    output
}

fn hostname_fallback() -> String {
    env::var("COMPUTERNAME")
        .or_else(|_| env::var("HOSTNAME"))
        .unwrap_or_default()
}

fn path_to_string(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().to_string()
}

fn to_string_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
