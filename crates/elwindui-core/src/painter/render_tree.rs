use super::command::RenderCommand;
use crate::base::{Point, Rect, Size};
use crate::ui::UIElementExt;
use std::collections::HashMap;
use std::rc::Weak;

/// One retained Visual node. `commands` are this Visual's own content; `children` is the visual tree.
pub struct RenderGroup {
    pub id: u64,
    pub is_dirty: bool,
    pub offset: Point,
    /// The arranged local extent. It is retained separately from `clip`: an unclipped Visual can
    /// still need to re-record its local commands when only its size changes.
    pub(crate) size: Size,
    pub clip: Option<Rect>,
    pub commands: Vec<RenderCommand>,
    pub children: Vec<RenderGroup>,
}

impl RenderGroup {
    pub fn new(id: u64, offset: Point, clip: Option<Rect>) -> Self {
        Self {
            id,
            is_dirty: true,
            offset,
            size: Size::default(),
            clip,
            commands: Vec::new(),
            children: Vec::new(),
        }
    }
}

/// Retained render tree plus lookup tables used by a host's deferred layout/render pass.
pub struct RenderTree {
    pub root: RenderGroup,
    pub group_paths: HashMap<u64, Vec<usize>>,
    pub visual_index: HashMap<u64, Weak<dyn UIElementExt>>,
}

impl RenderTree {
    pub(crate) fn with_root(root: RenderGroup) -> Self {
        Self {
            root,
            group_paths: HashMap::new(),
            visual_index: HashMap::new(),
        }
    }

    pub fn mark_dirty(&mut self, id: u64) -> bool {
        let Some(path) = self.group_paths.get(&id).cloned() else {
            return false;
        };
        let mut group = &mut self.root;
        for index in path {
            group = &mut group.children[index];
        }
        group.is_dirty = true;
        true
    }
}
