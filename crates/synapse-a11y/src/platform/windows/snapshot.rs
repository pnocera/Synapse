use std::time::{Duration, Instant};
use std::{
    collections::HashSet,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use synapse_core::{AccessibleNode, AccessibleSubtree, ElementId, UiaPattern, element_id};
use uiautomation::{
    UIAutomation, UIElement, UITreeWalker,
    core::UICacheRequest,
    types::{ElementMode, PropertyConditionFlags, TreeScope, UIProperty},
    variants::Variant,
};

use crate::{A11yError, A11yResult, ElementSearchScope, ids::runtime_id_hex};

use super::common::{
    TreeView, cached_hwnd, cached_patterns, cached_rect, cached_role, cached_runtime_id,
    create_cache_request, map_uia_error, non_empty, pattern_property, with_automation,
};

static SNAPSHOT_CACHE: Mutex<Option<SnapshotCache>> = Mutex::new(None);
static SNAPSHOT_DEPTH1_DEGRADED: AtomicBool = AtomicBool::new(false);
const RAW_SUPPLEMENT_DEPTH: u32 = 2;
const RAW_SUPPLEMENT_NODE_BUDGET: usize = 60;
// Packaged Notepad exposes these top-level menu items through raw
// name+ExpandCollapse search even when RawView child walking omits them.
const RAW_MENU_SUPPLEMENT_NAMES: [&str; 3] = ["File", "Edit", "View"];

struct SnapshotCache {
    requested_depth: u32,
    captured_at: Instant,
    tree: AccessibleSubtree,
}

struct SnapshotWalk<'a> {
    walker: &'a UITreeWalker,
    cache: &'a UICacheRequest,
    root_hwnd: i64,
}
pub fn snapshot(root: &UIElement, depth: u32) -> A11yResult<AccessibleSubtree> {
    if let Some(tree) = cached_snapshot(depth) {
        return Ok(tree);
    }

    with_automation(|automation| {
        let requested_depth = if depth > 1 && SNAPSHOT_DEPTH1_DEGRADED.load(Ordering::Relaxed) {
            1
        } else {
            depth
        };
        let start = Instant::now();
        let mut tree = snapshot_at_depth(automation, root, requested_depth)?;
        if depth > 1 && requested_depth > 1 && start.elapsed() > Duration::from_millis(25) {
            SNAPSHOT_DEPTH1_DEGRADED.store(true, Ordering::Relaxed);
            tree = snapshot_at_depth(automation, root, 1)?;
            tree.truncated = true;
        }
        if requested_depth < depth {
            tree.truncated = true;
        }
        if depth >= RAW_SUPPLEMENT_DEPTH {
            tree.truncated |= supplement_raw_pattern_nodes(automation, root, &mut tree.nodes)?;
            tree.max_depth = tree.max_depth.max(RAW_SUPPLEMENT_DEPTH);
        }
        store_snapshot(depth, &tree);
        Ok(tree)
    })
}

pub fn find_by_name_and_pattern(
    root: &UIElement,
    name: &str,
    pattern: UiaPattern,
    scope: ElementSearchScope,
) -> A11yResult<Option<AccessibleNode>> {
    if name.is_empty() {
        return Ok(None);
    }

    with_automation(|automation| {
        let cache = create_cache_request(automation, 0, ElementMode::Full, TreeView::Raw)?;
        let name_condition = automation
            .create_property_condition(
                UIProperty::Name,
                Variant::from(name),
                Some(PropertyConditionFlags::IgnoreCase),
            )
            .map_err(map_uia_error)?;
        let pattern_condition = automation
            .create_property_condition(pattern_property(pattern), Variant::from(true), None)
            .map_err(map_uia_error)?;
        let condition = automation
            .create_and_condition(name_condition, pattern_condition)
            .map_err(map_uia_error)?;
        let elements = root
            .find_all_build_cache(scope.into(), &condition, &cache)
            .map_err(map_uia_error)?;
        let root_hwnd = root
            .build_updated_cache(&cache)
            .ok()
            .and_then(|cached_root| cached_hwnd(&cached_root))
            .unwrap_or(0);

        elements
            .into_iter()
            .filter(|element| element.is_cached_enabled().unwrap_or(true))
            .map(|element| node_from_cached_element(&element, None, 0, root_hwnd, 0))
            .next()
            .transpose()
    })
}
fn snapshot_at_depth(
    automation: &UIAutomation,
    root: &UIElement,
    depth: u32,
) -> A11yResult<AccessibleSubtree> {
    let cache = create_cache_request(automation, 0, ElementMode::Full, TreeView::Raw)?;
    let walker = automation.get_raw_view_walker().map_err(map_uia_error)?;
    let cached_root = root.build_updated_cache(&cache).map_err(map_uia_error)?;
    let root_hwnd = cached_hwnd(&cached_root).unwrap_or(0);
    let mut nodes = Vec::new();
    let walk = SnapshotWalk {
        walker: &walker,
        cache: &cache,
        root_hwnd,
    };
    collect_nodes(&walk, &cached_root, None, 0, depth, &mut nodes)?;
    let root = nodes
        .first()
        .map(|node| node.element_id.clone())
        .ok_or_else(|| A11yError::ElementStale {
            detail: "snapshot root produced no UIA node".to_owned(),
        })?;
    Ok(AccessibleSubtree {
        root,
        nodes,
        max_depth: depth,
        truncated: false,
    })
}

fn supplement_raw_pattern_nodes(
    automation: &UIAutomation,
    root: &UIElement,
    nodes: &mut Vec<AccessibleNode>,
) -> A11yResult<bool> {
    let Some(root_id) = nodes.first().map(|node| node.element_id.clone()) else {
        return Ok(false);
    };
    let root_hwnd = root_id
        .parts()
        .map_err(|err| A11yError::InvalidElementId {
            detail: err.to_string(),
        })?
        .hwnd;
    let cache = create_cache_request(automation, 0, ElementMode::Full, TreeView::Raw)?;
    let mut seen: HashSet<ElementId> = nodes.iter().map(|node| node.element_id.clone()).collect();
    let mut truncated = false;
    for name in RAW_MENU_SUPPLEMENT_NAMES {
        let name_condition = automation
            .create_property_condition(
                UIProperty::Name,
                Variant::from(name),
                Some(PropertyConditionFlags::IgnoreCase),
            )
            .map_err(map_uia_error)?;
        let pattern_condition = automation
            .create_property_condition(
                UIProperty::IsExpandCollapsePatternAvailable,
                Variant::from(true),
                None,
            )
            .map_err(map_uia_error)?;
        let condition = automation
            .create_and_condition(name_condition, pattern_condition)
            .map_err(map_uia_error)?;
        let raw_elements = root
            .find_all_build_cache(TreeScope::Subtree, &condition, &cache)
            .map_err(map_uia_error)?;
        for element in raw_elements {
            if nodes.len() >= RAW_SUPPLEMENT_NODE_BUDGET {
                truncated = true;
                break;
            }
            let node = node_from_cached_element(
                &element,
                Some(root_id.clone()),
                RAW_SUPPLEMENT_DEPTH,
                root_hwnd,
                0,
            )?;
            if seen.insert(node.element_id.clone()) {
                nodes.push(node);
            }
        }
        if truncated {
            break;
        }
    }
    Ok(truncated)
}

fn cached_snapshot(depth: u32) -> Option<AccessibleSubtree> {
    let guard = SNAPSHOT_CACHE.lock().ok()?;
    let cache = guard.as_ref()?;
    let is_fresh =
        cache.requested_depth == depth && cache.captured_at.elapsed() <= Duration::from_millis(50);
    let tree = is_fresh.then(|| cache.tree.clone());
    drop(guard);
    tree
}

fn store_snapshot(depth: u32, tree: &AccessibleSubtree) {
    if let Ok(mut guard) = SNAPSHOT_CACHE.lock() {
        *guard = Some(SnapshotCache {
            requested_depth: depth,
            captured_at: Instant::now(),
            tree: tree.clone(),
        });
    }
}

pub(super) fn invalidate_snapshot_cache() {
    if let Ok(mut guard) = SNAPSHOT_CACHE.lock() {
        *guard = None;
    }
}

fn collect_nodes(
    walk: &SnapshotWalk<'_>,
    element: &UIElement,
    parent: Option<ElementId>,
    depth: u32,
    max_depth: u32,
    nodes: &mut Vec<AccessibleNode>,
) -> A11yResult<ElementId> {
    let children = if depth < max_depth {
        walk.walker
            .get_children_build_cache(element, walk.cache)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let node = node_from_cached_element(element, parent, depth, walk.root_hwnd, children.len())?;
    let node_id = node.element_id.clone();
    nodes.push(node);
    for child in children {
        collect_nodes(
            walk,
            &child,
            Some(node_id.clone()),
            depth + 1,
            max_depth,
            nodes,
        )?;
    }
    Ok(node_id)
}

fn node_from_cached_element(
    element: &UIElement,
    parent: Option<ElementId>,
    depth: u32,
    root_hwnd: i64,
    children_count: usize,
) -> A11yResult<AccessibleNode> {
    let runtime_id = cached_runtime_id(element)?;
    let runtime_id_hex = runtime_id_hex(&runtime_id);
    let hwnd = cached_hwnd(element)
        .filter(|value| *value != 0)
        .unwrap_or(root_hwnd);
    Ok(AccessibleNode {
        element_id: element_id(hwnd, &runtime_id_hex),
        parent,
        name: element.get_cached_name().unwrap_or_default(),
        role: cached_role(element),
        automation_id: non_empty(element.get_cached_automation_id().unwrap_or_default()),
        bbox: cached_rect(element),
        enabled: element.is_cached_enabled().unwrap_or(false),
        focused: element.has_cached_keyboard_focus().unwrap_or(false),
        patterns: cached_patterns(element),
        children_count: u32::try_from(children_count).unwrap_or(u32::MAX),
        depth,
    })
}
