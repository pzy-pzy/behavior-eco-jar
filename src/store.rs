//! 本地持久化。默认目录：%LOCALAPPDATA%\BehaviorEcoJar\
//!
//! - state.json   ：生态状态、暂停标志、窗口位置
//! - metrics.jsonl：每分钟一行聚合计数，保留 14 天
//!
//! 这里不会出现任何应用名、窗口标题或输入内容。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::Eco;
use crate::signature::Bucket;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Persisted {
    #[serde(default)]
    pub eco: Eco,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    /// 开机自启开关。None 表示还没设置过（按默认开启处理）。
    #[serde(default)]
    pub autostart: Option<bool>,
    /// 当日统计，重启后接着算。
    #[serde(default)]
    pub day_key: Option<String>,
    #[serde(default)]
    pub day_active_sec: Option<u64>,
    #[serde(default)]
    pub day_breaks: Option<u32>,
    /// 外观：0.8 迷你 / 1.0 紧凑 / 1.25 标准
    #[serde(default)]
    pub ui_scale: Option<f32>,
    /// 整体不透明度 0.5..1.0
    #[serde(default)]
    pub opacity: Option<f32>,
    /// 收进屏幕边缘的小圆点
    #[serde(default)]
    pub collapsed: Option<bool>,
}

impl Default for Persisted {
    fn default() -> Self {
        Self {
            eco: Eco::default(),
            paused: false,
            x: None,
            y: None,
            autostart: None,
            day_key: None,
            day_active_sec: None,
            day_breaks: None,
            ui_scale: None,
            opacity: None,
            collapsed: None,
        }
    }
}

/// 数据根目录。
///
/// 默认放在程序旁边（`data/`），而不是 `%LOCALAPPDATA%`：
/// 这台机器上的终端安全软件会拦截未签名程序写入用户目录，
/// 放在程序目录下更省事，也方便直接查看存档。
/// 需要时用 `ECOJAR_DATA_DIR` 覆盖。
pub fn data_root() -> PathBuf {
    if let Some(p) = std::env::var_os("ECOJAR_DATA_DIR") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.eq_ignore_ascii_case("debug") || name.eq_ignore_ascii_case("release") {
                if let Some(root) = dir.parent().and_then(|p| p.parent()) {
                    return root.join("data");
                }
            }
            return dir.join("data");
        }
    }
    PathBuf::from("data")
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new() -> Self {
        Self::at(data_root())
    }

    pub fn at(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    fn metrics_path(&self) -> PathBuf {
        self.dir.join("metrics.jsonl")
    }

    pub fn load(&self) -> Persisted {
        fs::read_to_string(self.state_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, p: &Persisted) {
        let path = self.state_path();
        let tmp = self.dir.join("state.json.tmp");
        if let Ok(text) = serde_json::to_string_pretty(p) {
            if fs::write(&tmp, text).is_ok() {
                let _ = fs::remove_file(&path);
                let _ = fs::rename(&tmp, &path);
            }
        }
    }

    pub fn append_bucket(&self, b: &Bucket) {
        let line = format!(
            "{{\"t\":{},\"a\":{},\"s\":{},\"i\":{}}}\n",
            b.t,
            b.active.min(60),
            b.switches,
            b.input.min(60)
        );
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.metrics_path())
        {
            let _ = f.write_all(line.as_bytes());
        }
    }

    /// 只保留最近 N 天。启动时调用一次即可。
    pub fn prune(&self, keep_days: u64, now_ms: u64) {
        let cutoff_min = now_ms / 60_000;
        let cutoff = cutoff_min.saturating_sub(keep_days * 24 * 60);
        let Ok(text) = fs::read_to_string(self.metrics_path()) else {
            return;
        };
        let kept: Vec<&str> = text
            .lines()
            .filter(|line| {
                serde_json::from_str::<BucketLine>(line)
                    .map(|b| b.t >= cutoff)
                    .unwrap_or(false)
            })
            .collect();
        if kept.len() != text.lines().count() {
            let _ = fs::write(self.metrics_path(), kept.join("\n") + "\n");
        }
    }

    /// 回收站式的“重置”：档案改名保留，方便事后查看。
    pub fn archive_state(&self) {
        let path = self.state_path();
        if path.exists() {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = fs::rename(&path, self.dir.join(format!("state.{stamp}.bak.json")));
        }
    }
}

#[derive(Deserialize)]
struct BucketLine {
    t: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("ecojar-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Store::at(dir)
    }

    #[test]
    fn state_roundtrip() {
        let store = temp_store("state");
        let mut p = Persisted::default();
        p.paused = true;
        p.x = Some(123);
        p.eco.active_minutes = 42.5;
        p.eco.stage = 1;
        store.save(&p);

        let q = store.load();
        assert!(q.paused);
        assert_eq!(q.x, Some(123));
        assert!((q.eco.active_minutes - 42.5).abs() < 0.001);
        assert_eq!(q.eco.stage, 1);
    }

    #[test]
    fn prune_drops_old_lines() {
        let store = temp_store("prune");
        let now = 1_700_000_000_000u64;
        let old = Bucket {
            t: now / 60_000 - 40 * 24 * 60,
            active: 60,
            switches: 1,
            input: 60,
        };
        let fresh = Bucket {
            t: now / 60_000,
            active: 30,
            switches: 2,
            input: 20,
        };
        store.append_bucket(&old);
        store.append_bucket(&fresh);
        store.prune(14, now);
        let text = fs::read_to_string(store.metrics_path()).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("\"s\":2"));
    }
}
