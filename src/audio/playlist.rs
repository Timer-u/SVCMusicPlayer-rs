use std::fs;
use std::path::{Path, PathBuf};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestOrigin {
    GameChat { sender: Option<String> },
    WebConsole,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    pub id: Uuid,
    pub title: String,
    pub raw_query: String,
    pub source: String,
    pub is_local_file: bool,
    pub http_headers: Option<std::collections::HashMap<String, String>>,
    pub source_bitrate_kbps: Option<f64>,
    pub duration_secs: Option<u64>,
    pub replay_gain_db: Option<f32>,
    pub origin: RequestOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayMode {
    ListLoop,   // 列表循环（默认）
    SingleLoop, // 单曲循环
    Shuffle,    // 随机播放（洗牌不重复）
    Sequential, // 顺序播放（播完最后一首停机）
}

impl PlayMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ListLoop => "ListLoop",
            Self::SingleLoop => "SingleLoop",
            Self::Shuffle => "Shuffle",
            Self::Sequential => "Sequential",
        }
    }

    pub fn display_cn(&self) -> &'static str {
        match self {
            Self::ListLoop => "列表循环",
            Self::SingleLoop => "单曲循环",
            Self::Shuffle => "随机播放",
            Self::Sequential => "顺序播放",
        }
    }

    pub fn from_input(s: &str) -> Option<Self> {
        let lower = s.trim().to_lowercase();
        match lower.as_str() {
            "loop" | "list" | "listloop" | "循环" | "列表循环" => Some(Self::ListLoop),
            "single" | "singleloop" | "单曲" | "单曲循环" => Some(Self::SingleLoop),
            "shuffle" | "random" | "rand" | "随机" | "随机播放" => Some(Self::Shuffle),
            "seq" | "sequential" | "order" | "顺序" | "顺序播放" => Some(Self::Sequential),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    TrackFinished,
    TrackFailed { track: TrackInfo, error: String },
}

#[derive(Serialize, Deserialize)]
struct PlaylistDiskData {
    tracks: Vec<TrackInfo>,
    mode: PlayMode,
}

pub struct PlaylistManager {
    pub tracks: Vec<TrackInfo>,
    pub current_index: Option<usize>,
    pub mode: PlayMode,
    shuffle_order: Vec<usize>,
    shuffle_pos: usize,
    persistence_path: PathBuf,
}

impl PlaylistManager {
    pub fn new(persistence_path: PathBuf) -> Self {
        let (tracks, mode) = Self::load_from_disk(&persistence_path);
        let current_index = if tracks.is_empty() { None } else { Some(0) };
        let mut mgr = Self {
            tracks,
            current_index,
            mode,
            shuffle_order: Vec::new(),
            shuffle_pos: 0,
            persistence_path,
        };
        mgr.rebuild_shuffle_order(None);
        mgr
    }

    fn load_from_disk(path: &Path) -> (Vec<TrackInfo>, PlayMode) {
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(data) = serde_json::from_str::<PlaylistDiskData>(&content) {
                    info!(
                        "[Playlist] 已从 {} 恢复持久化歌单 (共 {} 首曲目, 模式: {})",
                        path.display(),
                        data.tracks.len(),
                        data.mode.display_cn()
                    );
                    return (data.tracks, data.mode);
                }
            }
        }
        (Vec::new(), PlayMode::ListLoop)
    }

    pub fn save_to_disk(&self) {
        let data = PlaylistDiskData {
            tracks: self.tracks.clone(),
            mode: self.mode,
        };
        match serde_json::to_string_pretty(&data) {
            Ok(json) => {
                if let Err(e) = fs::write(&self.persistence_path, json) {
                    error!("[Playlist] 写入持久化歌单失败 {}: {:?}", self.persistence_path.display(), e);
                }
            }
            Err(e) => {
                error!("[Playlist] 序列化歌单失败: {:?}", e);
            }
        }
    }

    pub fn set_mode(&mut self, mode: PlayMode) {
        self.mode = mode;
        if mode == PlayMode::Shuffle {
            self.rebuild_shuffle_order(self.current_index);
        }
        self.save_to_disk();
    }

    fn rebuild_shuffle_order(&mut self, avoid_first: Option<usize>) {
        let len = self.tracks.len();
        if len == 0 {
            self.shuffle_order.clear();
            self.shuffle_pos = 0;
            return;
        }

        self.shuffle_order = (0..len).collect();
        let mut rng = rand::thread_rng();
        self.shuffle_order.shuffle(&mut rng);

        if let Some(avoid) = avoid_first {
            if len > 1 && self.shuffle_order[0] == avoid {
                self.shuffle_order.swap(0, len - 1);
            }
        }

        // 定位 shuffle_pos 到当前索引
        if let Some(cur) = self.current_index {
            if let Some(pos) = self.shuffle_order.iter().position(|&idx| idx == cur) {
                self.shuffle_pos = pos;
            } else {
                self.shuffle_pos = 0;
            }
        } else {
            self.shuffle_pos = 0;
        }
    }

    /// 追加曲目至末尾
    pub fn add_track(&mut self, track: TrackInfo) -> usize {
        self.tracks.push(track);
        let new_idx = self.tracks.len() - 1;
        if self.current_index.is_none() {
            self.current_index = Some(0);
        }
        self.rebuild_shuffle_order(None);
        self.save_to_disk();
        new_idx
    }

    /// 插入为下一首播放
    pub fn insert_next(&mut self, track: TrackInfo) -> usize {
        let insert_idx = match self.current_index {
            Some(cur) if cur < self.tracks.len() => cur + 1,
            _ => self.tracks.len(),
        };

        if insert_idx >= self.tracks.len() {
            self.tracks.push(track);
        } else {
            self.tracks.insert(insert_idx, track);
        }

        if self.current_index.is_none() {
            self.current_index = Some(0);
        }

        self.rebuild_shuffle_order(None);
        self.save_to_disk();
        insert_idx
    }

    /// 移除指定索引 (0-based) 的曲目，返回 (被移除的曲目, 是否曾是当前播放曲目)
    pub fn remove_track(&mut self, index_0based: usize) -> Option<(TrackInfo, bool)> {
        if index_0based >= self.tracks.len() {
            return None;
        }

        let was_current = self.current_index == Some(index_0based);
        let removed = self.tracks.remove(index_0based);

        if self.tracks.is_empty() {
            self.current_index = None;
        } else if let Some(cur) = self.current_index {
            if index_0based < cur {
                self.current_index = Some(cur - 1);
            } else if was_current {
                if cur >= self.tracks.len() {
                    match self.mode {
                        PlayMode::Sequential => self.current_index = None,
                        _ => self.current_index = Some(0),
                    }
                } else {
                    // 原 cur + 1 的元素已自动顶替到 cur
                    self.current_index = Some(cur);
                }
            }
        }

        self.rebuild_shuffle_order(None);
        self.save_to_disk();
        Some((removed, was_current))
    }

    /// 置顶指定曲目为下一首 (from_0based -> cur + 1)
    pub fn move_to_next(&mut self, from_0based: usize) -> anyhow::Result<usize> {
        if from_0based >= self.tracks.len() {
            anyhow::bail!("曲目序号越界");
        }

        match self.current_index {
            None => {
                let track = self.tracks.remove(from_0based);
                self.tracks.insert(0, track);
                self.current_index = Some(0);
                self.rebuild_shuffle_order(None);
                self.save_to_disk();
                Ok(0)
            }
            Some(cur) => {
                if from_0based == cur || from_0based == cur + 1 {
                    return Ok(from_0based);
                }

                let track = self.tracks.remove(from_0based);
                let target_idx = if from_0based < cur {
                    self.current_index = Some(cur - 1);
                    cur
                } else {
                    cur + 1
                };

                self.tracks.insert(target_idx, track);
                self.rebuild_shuffle_order(None);
                self.save_to_disk();
                Ok(target_idx)
            }
        }
    }

    /// 切到下一首曲目
    pub fn advance_next(&mut self, is_manual_skip: bool) -> Option<TrackInfo> {
        let len = self.tracks.len();
        if len == 0 {
            return None;
        }

        let cur = self.current_index.unwrap_or(0);

        match self.mode {
            PlayMode::SingleLoop => {
                if is_manual_skip {
                    self.current_index = Some((cur + 1) % len);
                } else {
                    self.current_index = Some(cur);
                }
            }
            PlayMode::ListLoop => {
                self.current_index = Some((cur + 1) % len);
            }
            PlayMode::Sequential => {
                if cur + 1 < len {
                    self.current_index = Some(cur + 1);
                } else {
                    self.current_index = None;
                    return None;
                }
            }
            PlayMode::Shuffle => {
                if self.shuffle_order.len() != len {
                    self.rebuild_shuffle_order(None);
                }

                self.shuffle_pos += 1;
                if self.shuffle_pos >= self.shuffle_order.len() {
                    self.rebuild_shuffle_order(Some(cur));
                    self.shuffle_pos = 0;
                }

                self.current_index = Some(self.shuffle_order[self.shuffle_pos]);
            }
        }

        self.current_index.and_then(|idx| self.tracks.get(idx).cloned())
    }

    /// 切到上一首曲目
    pub fn advance_prev(&mut self) -> Option<TrackInfo> {
        let len = self.tracks.len();
        if len == 0 {
            return None;
        }

        let cur = self.current_index.unwrap_or(0);

        match self.mode {
            PlayMode::Shuffle => {
                if self.shuffle_order.len() != len {
                    self.rebuild_shuffle_order(None);
                }

                if self.shuffle_pos == 0 {
                    self.shuffle_pos = self.shuffle_order.len().saturating_sub(1);
                } else {
                    self.shuffle_pos -= 1;
                }

                self.current_index = Some(self.shuffle_order[self.shuffle_pos]);
            }
            _ => {
                let prev = if cur == 0 { len - 1 } else { cur - 1 };
                self.current_index = Some(prev);
            }
        }

        self.current_index.and_then(|idx| self.tracks.get(idx).cloned())
    }

    /// 跳转到指定 0-based 索引
    pub fn jump_to(&mut self, index_0based: usize) -> Option<TrackInfo> {
        if index_0based < self.tracks.len() {
            self.current_index = Some(index_0based);
            if self.mode == PlayMode::Shuffle {
                if let Some(pos) = self.shuffle_order.iter().position(|&i| i == index_0based) {
                    self.shuffle_pos = pos;
                }
            }
            self.tracks.get(index_0based).cloned()
        } else {
            None
        }
    }

    /// 获取当前曲目
    pub fn get_current_track(&self) -> Option<TrackInfo> {
        self.current_index.and_then(|idx| self.tracks.get(idx).cloned())
    }

    /// 清空歌单
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current_index = None;
        self.shuffle_order.clear();
        self.shuffle_pos = 0;
        self.save_to_disk();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_track(title: &str) -> TrackInfo {
        TrackInfo {
            id: Uuid::new_v4(),
            title: title.to_string(),
            raw_query: title.to_string(),
            source: format!("test://{}", title),
            is_local_file: true,
            http_headers: None,
            source_bitrate_kbps: None,
            duration_secs: Some(180),
            replay_gain_db: None,
            origin: RequestOrigin::Internal,
        }
    }

    #[test]
    fn test_playlist_modes() {
        let temp_file = PathBuf::from("target/test_playlist.json");
        let _ = fs::remove_file(&temp_file);
        let mut pl = PlaylistManager::new(temp_file.clone());

        pl.add_track(dummy_track("Song 1"));
        pl.add_track(dummy_track("Song 2"));
        pl.add_track(dummy_track("Song 3"));

        assert_eq!(pl.tracks.len(), 3);
        assert_eq!(pl.current_index, Some(0));

        // 1. ListLoop
        pl.set_mode(PlayMode::ListLoop);
        assert_eq!(pl.advance_next(false).unwrap().title, "Song 2");
        assert_eq!(pl.advance_next(false).unwrap().title, "Song 3");
        assert_eq!(pl.advance_next(false).unwrap().title, "Song 1"); // wrapped

        // 2. SingleLoop
        pl.set_mode(PlayMode::SingleLoop);
        assert_eq!(pl.advance_next(false).unwrap().title, "Song 1"); // auto repeat
        assert_eq!(pl.advance_next(true).unwrap().title, "Song 2");  // manual skip

        // 3. Sequential
        pl.set_mode(PlayMode::Sequential);
        assert_eq!(pl.advance_next(false).unwrap().title, "Song 3");
        assert!(pl.advance_next(false).is_none()); // reached end

        let _ = fs::remove_file(&temp_file);
    }

    #[test]
    fn test_playlist_operations() {
        let temp_file = PathBuf::from("target/test_pl_ops.json");
        let _ = fs::remove_file(&temp_file);
        let mut pl = PlaylistManager::new(temp_file.clone());

        pl.add_track(dummy_track("A"));
        pl.add_track(dummy_track("B"));
        pl.add_track(dummy_track("C"));

        // Insert next at cur=0 (A): should insert at 1
        pl.insert_next(dummy_track("A.5"));
        assert_eq!(pl.tracks[1].title, "A.5");

        // Move to next: move C (index 3) to cur+1 (index 1)
        let new_idx = pl.move_to_next(3).unwrap();
        assert_eq!(new_idx, 1);
        assert_eq!(pl.tracks[1].title, "C");

        // Remove track
        let (removed, was_cur) = pl.remove_track(0).unwrap();
        assert_eq!(removed.title, "A");
        assert!(was_cur);
        // cur was 0, now C is at 0
        assert_eq!(pl.current_index, Some(0));
        assert_eq!(pl.tracks[0].title, "C");

        let _ = fs::remove_file(&temp_file);
    }
}
