//! 行为签名：把分钟级的工作行为聚合成五个可解释的维度。
//!
//! 只依赖聚合计数（活跃秒数、前台切换次数、出现过的应用数），
//! 不保存任何窗口标题或输入内容。

use serde::{Deserialize, Serialize};

/// 一个完整的分钟桶。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Bucket {
    /// Unix 分钟序号。
    pub t: u64,
    /// 该分钟内处于活跃状态的秒数（0..=60）。
    pub active: u32,
    /// 该分钟内前台应用切换次数。
    pub switches: u32,
    /// 该分钟内有键鼠输入的秒数（0..=60）。
    #[serde(default)]
    pub input: u32,
}

/// 0..1 归一化。
pub fn clamp01(x: f32) -> f32 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Signature {
    /// 连续投入的程度。
    pub focus: f32,
    /// 工作与休息是否有节律。
    pub rhythm: f32,
    /// 是否平和、少碎切。
    pub calm: f32,
    /// 是否过劳（负向）。
    pub load: f32,
    /// 线索是否多样。
    pub variety: f32,
    /// 窗口内活跃占比。
    pub active_ratio: f32,
    /// 窗口内有输入的秒数占比。远程桌面里也准确。
    pub input_ratio: f32,
    /// 探索度是否可测（只有一个前台窗口时不可测）。
    pub variety_known: bool,
    /// 每分钟切换次数。
    pub switch_rate: f32,
    /// 当前连续活跃分钟数。
    pub streak_min: f32,
    /// 距离上次真正休息的分钟数。
    pub rest_min: f32,
    /// 窗口覆盖的分钟数。
    pub covered_min: u32,
}

impl Default for Signature {
    fn default() -> Self {
        Self {
            focus: 0.0,
            rhythm: 0.35,
            calm: 0.6,
            load: 0.0,
            variety: 0.0,
            active_ratio: 0.0,
            input_ratio: 0.0,
            variety_known: false,
            switch_rate: 0.0,
            streak_min: 0.0,
            rest_min: 0.0,
            covered_min: 0,
        }
    }
}

pub const WINDOW_MIN: u64 = 45;
/// 一个桶活跃秒数低于该值就算“在休息”。
const REST_ACTIVE_SEC: u32 = 15;

/// 从分钟桶计算签名。
pub fn compute(
    buckets: &[Bucket],
    now_ms: u64,
    current_streak_sec: u64,
    last_break_ms: u64,
    distinct_apps: usize,
) -> Signature {
    let now_min = now_ms / 60_000;
    let from = now_min.saturating_sub(WINDOW_MIN - 1);

    let window: Vec<&Bucket> = buckets.iter().filter(|b| b.t >= from).collect();
    if window.is_empty() {
        return Signature {
            rest_min: ((now_ms.saturating_sub(last_break_ms)) / 60_000) as f32,
            ..Signature::default()
        };
    }

    let first_min = window.iter().map(|b| b.t).min().unwrap_or(now_min);
    let covered = (now_min.saturating_sub(first_min) + 1).min(WINDOW_MIN) as u32;
    let covered_f = covered.max(1) as f32;

    let active_sec: u32 = window.iter().map(|b| b.active).sum();
    let input_sec: u32 = window.iter().map(|b| b.input).sum();
    let switches: u32 = window.iter().map(|b| b.switches).sum();
    let busy_minutes = window.iter().filter(|b| b.switches >= 5).count() as f32;

    let active_ratio = clamp01(active_sec as f32 / (covered_f * 60.0));
    let input_ratio = clamp01(input_sec as f32 / (covered_f * 60.0));
    let switch_rate = switches as f32 / covered_f;
    let burst_ratio = busy_minutes / covered_f;

    let streak_min = current_streak_sec as f32 / 60.0;

    // 专注：在场（活跃）× 真的在敲（输入密度）× 低切换 × 连续时长。
    // 只看活跃占比的话，读文档和写代码会被算成一样；输入密度能把两者分开。
    let engagement = 0.45 * active_ratio + 0.55 * input_ratio;
    let switch_penalty = 1.0 / (1.0 + switch_rate / 8.0);
    let streak_factor = 0.35 + 0.65 * clamp01(streak_min / 20.0);
    let focus = clamp01(engagement * switch_penalty * streak_factor);

    // 平静：切换少、且没有大量“碎切分钟”。
    let calm = clamp01((1.0 - switch_rate / 12.0) * (1.0 - burst_ratio * 0.7));

    // 探索：出现过的应用数 + 切换频率。
    // 只有一个前台窗口时（例如远程桌面里工作）这个维度不可测，给中性值。
    let variety_known = distinct_apps > 1 || switch_rate >= 0.5;
    let app_term = if distinct_apps == 0 {
        0.0
    } else {
        clamp01(distinct_apps as f32 / 8.0)
    };
    let variety = if variety_known {
        clamp01(0.65 * app_term + 0.35 * clamp01(switch_rate / 6.0))
    } else {
        0.5
    };

    // 节律：休息间隔是否稳定。
    let rhythm = rest_rhythm(&window, now_min);

    // 负荷：连续工作时长 + 迟迟不休息。
    let since_break_min = (now_ms.saturating_sub(last_break_ms)) as f32 / 60_000.0;
    let load = clamp01(
        (streak_min / 110.0).max(since_break_min / 210.0) * (0.55 + 0.45 * active_ratio),
    );

    Signature {
        focus,
        rhythm,
        calm,
        load,
        variety,
        active_ratio,
        input_ratio,
        variety_known,
        switch_rate,
        streak_min,
        rest_min: since_break_min,
        covered_min: covered,
    }
}

/// 用“休息段”的间隔稳定性衡量节律。
fn rest_rhythm(window: &[&Bucket], now_min: u64) -> f32 {
    let mut starts: Vec<u64> = Vec::new();
    let mut run = 0u32;
    let mut prev_rest = false;

    for b in window {
        let rest = b.active < REST_ACTIVE_SEC;
        if rest {
            if !prev_rest {
                run = 0;
            }
            run += 1;
            if run == 3 {
                starts.push(b.t);
            }
        }
        prev_rest = rest;
    }

    if starts.len() < 2 {
        return 0.35;
    }

    let mut intervals: Vec<f32> = Vec::new();
    for pair in starts.windows(2) {
        intervals.push((pair[1] - pair[0]) as f32);
    }
    // 忽略过短的抖动间隔。
    intervals.retain(|v| *v >= 5.0);
    if intervals.len() < 1 {
        return 0.35;
    }

    let n = intervals.len() as f32;
    let mean = intervals.iter().sum::<f32>() / n;
    if mean <= 0.0 {
        return 0.35;
    }
    let var = intervals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n;
    let cv = var.sqrt() / mean;

    // 最近一次休息离现在太久，也说明节律没维持住。
    let last_start = *starts.last().unwrap();
    let stale = ((now_min.saturating_sub(last_start)) as f32 / 120.0).clamp(0.0, 1.0);

    clamp01((1.0 - cv / 1.2) * (1.0 - stale * 0.5))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;
    const T0: u64 = 1_700_000_000 * MIN;

    fn steady(minutes: u64, active: u32, switches: u32) -> Vec<Bucket> {
        (0..minutes)
            .map(|i| Bucket {
                t: T0 / MIN + i,
                active,
                switches,
                input: active,
            })
            .collect()
    }

    fn reading(minutes: u64) -> Vec<Bucket> {
        (0..minutes)
            .map(|i| Bucket {
                t: T0 / MIN + i,
                active: 60,
                switches: 0,
                input: 0,
            })
            .collect()
    }

    #[test]
    fn empty_window_is_sane() {
        let s = compute(&[], T0, 0, T0, 0);
        assert!(s.focus.is_finite() && s.calm.is_finite() && s.rhythm.is_finite());
        assert_eq!(s.covered_min, 0);
    }

    #[test]
    fn deep_focus_scores_high() {
        let buckets = steady(45, 60, 0);
        let now = T0 + 45 * MIN;
        let s = compute(&buckets, now, 45 * 60, now, 2);
        assert!(s.active_ratio > 0.95, "active_ratio={}", s.active_ratio);
        assert!(s.focus > 0.8, "focus={}", s.focus);
        assert!(s.load > 0.2, "load={}", s.load);
    }

    #[test]
    fn fragmentation_lowers_focus_and_calm() {
        let buckets = steady(45, 60, 12);
        let now = T0 + 45 * MIN;
        let s = compute(&buckets, now, 45 * 60, now, 9);
        assert!(s.calm < 0.2, "calm={}", s.calm);
        assert!(s.focus < 0.5, "focus={}", s.focus);
        assert!(s.variety > 0.7, "variety={}", s.variety);
    }

    #[test]
    fn regular_breaks_give_rhythm() {
        let mut buckets = Vec::new();
        for block in 0..4 {
            for i in 0..20 {
                buckets.push(Bucket {
                    t: T0 / MIN + block * 30 + i,
                    active: 60,
                    switches: 1,
                    input: 60,
                });
            }
            for i in 20..30 {
                buckets.push(Bucket {
                    t: T0 / MIN + block * 30 + i,
                    active: 0,
                    switches: 0,
                    input: 0,
                });
            }
        }
        let now = T0 + 120 * MIN;
        let s = compute(&buckets, now, 10 * 60, now - 10 * MIN, 3);
        assert!(s.rhythm > 0.6, "rhythm={}", s.rhythm);
    }

    #[test]
    fn reading_scores_lower_than_typing() {
        let now = T0 + 45 * MIN;
        let typing = compute(&steady(45, 60, 0), now, 45 * 60, now, 2);
        let reading = compute(&reading(45), now, 45 * 60, now, 2);
        assert!(
            typing.focus > reading.focus + 0.2,
            "typing={} reading={}",
            typing.focus,
            reading.focus
        );
        assert!(reading.focus > 0.2, "reading={}", reading.focus);
    }

    #[test]
    fn single_window_means_variety_unknown() {
        let now = T0 + 45 * MIN;
        let s = compute(&steady(45, 60, 0), now, 45 * 60, now, 1);
        assert!(!s.variety_known);
        assert!((s.variety - 0.5).abs() < 0.001);
    }
    #[test]
    fn no_breaks_means_high_load() {
        let buckets = steady(45, 60, 1);
        let now = T0 + 45 * MIN;
        let s = compute(&buckets, now, 200 * 60, T0 - 60 * MIN, 2);
        assert!(s.load > 0.7, "load={}", s.load);
        assert!(s.rest_min > 100.0);
    }
}
