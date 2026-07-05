pub mod panel;

use std::cmp::Ordering;

use crate::source::{CAL_READ_MAX, ParamWrite, ValueRead, VarDescriptor};

#[derive(Debug, Clone)]
pub enum DescriptorEntry {
    Var {
        label: String,
        full_name: String,
        index: usize,
    },
    Group {
        label: String,
        full_name: String,
        members: Vec<DescriptorEntry>,
    },
}

impl DescriptorEntry {
    fn label(&self) -> &str {
        match self {
            Self::Var { label, .. } | Self::Group { label, .. } => label,
        }
    }

    fn full_name(&self) -> &str {
        match self {
            Self::Var { full_name, .. } | Self::Group { full_name, .. } => full_name,
        }
    }

    pub fn flatten_names(&self, out: &mut Vec<String>) {
        match self {
            Self::Var { full_name, .. } => out.push(full_name.clone()),
            Self::Group { members, .. } => {
                for member in members {
                    member.flatten_names(out);
                }
            }
        }
    }

    pub fn leaf_indexes(&self, out: &mut Vec<usize>) {
        match self {
            Self::Var { index, .. } => out.push(*index),
            Self::Group { members, .. } => {
                for member in members {
                    member.leaf_indexes(out);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct WatchEntry {
    pub var_name: String,
    pub descriptor_index: usize,
    pub write_buf: String,
}

#[derive(Default)]
pub struct InspectorState {
    pub entries: Vec<DescriptorEntry>,
    pub system_entries: Vec<DescriptorEntry>,
    pub descriptors: Vec<VarDescriptor>,
    pub pinned: Vec<usize>,
    pub watch_vars: Vec<WatchEntry>,
    pub values: Vec<Option<f64>>,
}

impl InspectorState {
    pub fn set_descriptors(&mut self, descriptors: Vec<VarDescriptor>) {
        let pinned_names: Vec<String> = self
            .pinned
            .iter()
            .filter_map(|&index| self.descriptors.get(index).map(|d| d.name.clone()))
            .collect();
        let watch_names: Vec<(String, String)> = self
            .watch_vars
            .iter()
            .map(|watch| (watch.var_name.clone(), watch.write_buf.clone()))
            .collect();

        self.entries = build_descriptor_tree_filtered(&descriptors, VarDescriptor::is_user);
        self.system_entries =
            build_descriptor_tree_filtered(&descriptors, |descriptor| !descriptor.is_user());
        self.values = vec![None; descriptors.len()];
        self.descriptors = descriptors;

        self.pinned = pinned_names
            .iter()
            .filter_map(|name| self.index_by_name(name))
            .collect();
        self.watch_vars = watch_names
            .into_iter()
            .filter_map(|(name, write_buf)| {
                self.index_by_name(&name)
                    .map(|descriptor_index| WatchEntry {
                        var_name: name,
                        descriptor_index,
                        write_buf,
                    })
            })
            .collect();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.system_entries.clear();
        self.descriptors.clear();
        self.pinned.clear();
        self.watch_vars.clear();
        self.values.clear();
    }

    pub fn index_by_name(&self, name: &str) -> Option<usize> {
        self.descriptors
            .iter()
            .position(|descriptor| descriptor.name == name)
    }

    pub fn descriptor_by_name(&self, name: &str) -> Option<&VarDescriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.name == name)
    }

    pub fn is_system_variable_index(&self, index: usize) -> bool {
        self.descriptors
            .get(index)
            .is_some_and(|descriptor| !descriptor.is_user())
    }

    pub fn is_system_variable_name(&self, name: &str) -> bool {
        self.descriptor_by_name(name)
            .is_some_and(|descriptor| !descriptor.is_user())
    }

    pub fn system_var_names(&self) -> Vec<String> {
        self.descriptors
            .iter()
            .filter(|descriptor| !descriptor.is_user())
            .map(|descriptor| descriptor.name.clone())
            .collect()
    }

    pub fn value_by_name(&self, name: &str) -> Option<f64> {
        self.index_by_name(name)
            .and_then(|index| self.values.get(index).copied().flatten())
    }

    pub fn var_names(&self) -> Vec<String> {
        self.descriptors
            .iter()
            .map(|descriptor| descriptor.name.clone())
            .collect()
    }

    pub fn display_values(&self) -> Vec<f64> {
        self.values
            .iter()
            .map(|value| value.unwrap_or(f64::NAN))
            .collect()
    }

    pub fn update_values(&mut self, indexes: &[usize], values: Vec<u32>) {
        for (&index, bits) in indexes.iter().zip(values.into_iter()) {
            let Some(descriptor) = self.descriptors.get(index) else {
                continue;
            };
            let Some(raw) = descriptor.var.ty.decode(&bits.to_le_bytes()) else {
                continue;
            };
            if let Some(slot) = self.values.get_mut(index) {
                *slot = Some(raw);
            }
        }
    }

    pub fn watched_indexes(&self) -> Vec<usize> {
        let mut indexes = self.pinned.clone();
        for watch in &self.watch_vars {
            if !indexes.contains(&watch.descriptor_index) {
                indexes.push(watch.descriptor_index);
            }
        }
        indexes.sort_unstable();
        indexes
    }

    pub fn read_batches(&self) -> Vec<Vec<ValueRead>> {
        let indexes = self.watched_indexes();
        if indexes.is_empty() {
            return Vec::new();
        }

        indexes
            .chunks(CAL_READ_MAX)
            .map(|chunk| {
                chunk
                    .iter()
                    .filter_map(|&descriptor_index| {
                        self.descriptors
                            .get(descriptor_index)
                            .map(|descriptor| ValueRead {
                                descriptor_index,
                                var: descriptor.var,
                            })
                    })
                    .collect()
            })
            .filter(|batch: &Vec<ValueRead>| !batch.is_empty())
            .collect()
    }

    pub fn param_write_for(&self, descriptor_index: usize, value: f64) -> Option<ParamWrite> {
        let descriptor = self.descriptors.get(descriptor_index)?;
        if !descriptor.is_parameter() {
            return None;
        }
        Some(ParamWrite {
            var: descriptor.var,
            value_bits: descriptor.var.ty.encode_value_bits(value),
        })
    }
}

#[cfg(test)]
pub fn build_descriptor_tree(descriptors: &[VarDescriptor]) -> Vec<DescriptorEntry> {
    build_descriptor_tree_filtered(descriptors, |_| true)
}

fn build_descriptor_tree_filtered(
    descriptors: &[VarDescriptor],
    include: impl Fn(&VarDescriptor) -> bool,
) -> Vec<DescriptorEntry> {
    let mut root = Vec::<DescriptorEntry>::new();
    for (index, descriptor) in descriptors.iter().enumerate() {
        if include(descriptor) {
            insert_descriptor(&mut root, &descriptor.name, index);
        }
    }
    sort_descriptor_entries(&mut root);
    root
}

fn sort_descriptor_entries(entries: &mut [DescriptorEntry]) {
    for entry in entries.iter_mut() {
        if let DescriptorEntry::Group { members, .. } = entry {
            sort_descriptor_entries(members);
        }
    }
    entries.sort_by(compare_descriptor_entries);
}

fn compare_descriptor_entries(left: &DescriptorEntry, right: &DescriptorEntry) -> Ordering {
    natural_descriptor_cmp(left.label(), right.label())
        .then_with(|| natural_descriptor_cmp(left.full_name(), right.full_name()))
}

fn natural_descriptor_cmp(left: &str, right: &str) -> Ordering {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut left_offset = 0;
    let mut right_offset = 0;

    while left_offset < left.len() && right_offset < right.len() {
        let left_token = next_sort_token(left, left_offset);
        let right_token = next_sort_token(right, right_offset);

        match left_token.cmp_value(right_token) {
            Ordering::Equal => {
                left_offset = left_token.end;
                right_offset = right_token.end;
            }
            ordering => return ordering,
        }
    }

    left.len().cmp(&right.len())
}

#[derive(Clone, Copy)]
struct SortToken<'a> {
    class: SortTokenClass,
    bytes: &'a [u8],
    end: usize,
}

impl SortToken<'_> {
    fn cmp_value(self, other: Self) -> Ordering {
        self.class
            .cmp(&other.class)
            .then_with(|| match (self.class, other.class) {
                (SortTokenClass::Digit, SortTokenClass::Digit) => {
                    compare_digit_runs(self.bytes, other.bytes)
                }
                (SortTokenClass::Letter, SortTokenClass::Letter) => {
                    compare_ascii_case_insensitive(self.bytes, other.bytes)
                }
                _ => self.bytes.cmp(other.bytes),
            })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SortTokenClass {
    Digit,
    Letter,
    Other,
}

fn next_sort_token(bytes: &[u8], offset: usize) -> SortToken<'_> {
    let start = if bytes[offset] == b'['
        && offset + 1 < bytes.len()
        && bytes[offset + 1].is_ascii_digit()
    {
        offset + 1
    } else {
        offset
    };
    let class = sort_token_class(bytes[start]);
    let mut end = start + 1;
    while end < bytes.len() && sort_token_class(bytes[end]) == class {
        end += 1;
    }
    let value_end = end;
    if start != offset && end < bytes.len() && bytes[end] == b']' {
        end += 1;
    }

    SortToken {
        class,
        bytes: &bytes[start..value_end],
        end,
    }
}

fn sort_token_class(byte: u8) -> SortTokenClass {
    if byte.is_ascii_digit() {
        SortTokenClass::Digit
    } else if byte.is_ascii_alphabetic() {
        SortTokenClass::Letter
    } else {
        SortTokenClass::Other
    }
}

fn compare_digit_runs(left: &[u8], right: &[u8]) -> Ordering {
    let left_digits = trim_leading_zeroes(left);
    let right_digits = trim_leading_zeroes(right);
    left_digits
        .len()
        .cmp(&right_digits.len())
        .then_with(|| left_digits.cmp(right_digits))
        .then_with(|| left.len().cmp(&right.len()))
}

fn trim_leading_zeroes(bytes: &[u8]) -> &[u8] {
    let trimmed = bytes
        .iter()
        .position(|byte| *byte != b'0')
        .map(|pos| &bytes[pos..])
        .unwrap_or(&[]);
    if trimmed.is_empty() { b"0" } else { trimmed }
}

fn compare_ascii_case_insensitive(left: &[u8], right: &[u8]) -> Ordering {
    left.iter()
        .map(u8::to_ascii_lowercase)
        .cmp(right.iter().map(u8::to_ascii_lowercase))
        .then_with(|| left.cmp(right))
}

#[derive(Debug)]
struct PathPart<'a> {
    label: &'a str,
    full_name: String,
}

fn insert_descriptor(entries: &mut Vec<DescriptorEntry>, name: &str, index: usize) {
    let parts = descriptor_path(name);
    if parts.is_empty() {
        entries.push(DescriptorEntry::Var {
            label: name.to_owned(),
            full_name: name.to_owned(),
            index,
        });
        return;
    }
    insert_parts(entries, &parts, index);
}

fn descriptor_path(name: &str) -> Vec<PathPart<'_>> {
    let bytes = name.as_bytes();
    let mut parts = Vec::new();
    let mut prefix = String::new();
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset] == b'.' {
            offset += 1;
            continue;
        }
        let start = offset;
        let is_index = bytes[offset] == b'[';
        if is_index {
            offset += 1;
            while offset < bytes.len() && bytes[offset] != b']' {
                offset += 1;
            }
            if offset < bytes.len() {
                offset += 1;
            }
        } else {
            while offset < bytes.len() && bytes[offset] != b'.' && bytes[offset] != b'[' {
                offset += 1;
            }
        }
        let label = &name[start..offset];
        if label.is_empty() {
            continue;
        }
        if !prefix.is_empty() && !is_index {
            prefix.push('.');
        }
        prefix.push_str(label);
        parts.push(PathPart {
            label,
            full_name: prefix.clone(),
        });
    }
    parts
}

fn insert_parts(entries: &mut Vec<DescriptorEntry>, parts: &[PathPart<'_>], index: usize) {
    let label = parts[0].label.to_owned();
    let full_name = parts[0].full_name.clone();

    if parts.len() == 1 {
        entries.push(DescriptorEntry::Var {
            label,
            full_name,
            index,
        });
        return;
    }

    let group_index = entries.iter().position(|entry| {
        matches!(
            entry,
            DescriptorEntry::Group {
                full_name: existing,
                ..
            } if existing == &full_name
        )
    });
    let group_index = match group_index {
        Some(group_index) => group_index,
        None => {
            entries.push(DescriptorEntry::Group {
                label,
                full_name: full_name.clone(),
                members: Vec::new(),
            });
            entries.len() - 1
        }
    };

    if let DescriptorEntry::Group { members, .. } = &mut entries[group_index] {
        insert_parts(members, &parts[1..], index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{VarRef, VarType};

    fn descriptor(name: &str) -> VarDescriptor {
        descriptor_with_kind(name, 0)
    }

    fn descriptor_with_kind(name: &str, kind: u16) -> VarDescriptor {
        VarDescriptor {
            name: name.to_owned(),
            var: VarRef {
                addr: 0,
                ty: VarType::F32,
            },
            kind,
            prescaler: 1,
        }
    }

    #[test]
    fn descriptor_tree_groups_dot_names() {
        let tree = build_descriptor_tree(&[
            descriptor("plant.current_a"),
            descriptor("plant.current_b"),
            descriptor("status"),
        ]);
        assert_eq!(tree.len(), 2);
        let DescriptorEntry::Group { members, .. } = &tree[0] else {
            panic!("first entry should be a group");
        };
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn descriptor_tree_groups_array_indexes() {
        let tree = build_descriptor_tree(&[
            descriptor("offset[0]"),
            descriptor("offset[1]"),
            descriptor("offset[2]"),
            descriptor("trace.err[0]"),
            descriptor("trace.err[1]"),
        ]);

        let DescriptorEntry::Group {
            label,
            full_name,
            members,
        } = &tree[0]
        else {
            panic!("offset should be a group");
        };
        assert_eq!(label, "offset");
        assert_eq!(full_name, "offset");
        assert_eq!(members.len(), 3);
        assert!(matches!(
            &members[0],
            DescriptorEntry::Var {
                label,
                full_name,
                ..
            } if label == "[0]" && full_name == "offset[0]"
        ));

        let DescriptorEntry::Group { members, .. } = &tree[1] else {
            panic!("trace should be a group");
        };
        let DescriptorEntry::Group {
            label,
            full_name,
            members,
        } = &members[0]
        else {
            panic!("trace.err should be a group");
        };
        assert_eq!(label, "err");
        assert_eq!(full_name, "trace.err");
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn descriptor_tree_sorts_root_and_group_members_naturally() {
        let tree = build_descriptor_tree(&[
            descriptor("motor.z"),
            descriptor("10_global"),
            descriptor("motor.a10"),
            descriptor("alpha"),
            descriptor("2_global"),
            descriptor("motor.a2"),
            descriptor("motor.a1"),
            descriptor("1_global"),
        ]);

        let mut names = Vec::new();
        for entry in &tree {
            entry.flatten_names(&mut names);
        }

        assert_eq!(
            names,
            [
                "1_global",
                "2_global",
                "10_global",
                "alpha",
                "motor.a1",
                "motor.a2",
                "motor.a10",
                "motor.z"
            ]
        );
    }

    #[test]
    fn descriptor_tree_sorts_array_indexes_numerically() {
        let tree = build_descriptor_tree(&[
            descriptor("offset[10]"),
            descriptor("offset[2]"),
            descriptor("offset[0]"),
            descriptor("offset[1]"),
        ]);

        let DescriptorEntry::Group { members, .. } = &tree[0] else {
            panic!("offset should be a group");
        };
        let labels: Vec<_> = members.iter().map(DescriptorEntry::label).collect();

        assert_eq!(labels, ["[0]", "[1]", "[2]", "[10]"]);
    }

    #[test]
    fn inspector_separates_user_and_system_trees() {
        let mut state = InspectorState::default();
        state.set_descriptors(vec![
            descriptor_with_kind("sys_state", 0x0002),
            descriptor_with_kind("offset[0]", 0x0007),
            descriptor_with_kind("offset[1]", 0x0007),
        ]);

        let mut user_names = Vec::new();
        for entry in &state.entries {
            entry.flatten_names(&mut user_names);
        }
        let mut system_names = Vec::new();
        for entry in &state.system_entries {
            entry.flatten_names(&mut system_names);
        }

        assert_eq!(user_names, ["offset[0]", "offset[1]"]);
        assert_eq!(system_names, ["sys_state"]);
        assert!(state.is_system_variable_name("sys_state"));
        assert!(state.is_system_variable_index(0));
        assert_eq!(state.system_var_names(), ["sys_state"]);
        assert!(!state.is_system_variable_name("offset[0]"));
    }

    #[test]
    fn read_batches_include_watched_indexes() {
        let mut state = InspectorState::default();
        state.set_descriptors(vec![
            descriptor("a"),
            descriptor("b"),
            descriptor("c"),
            descriptor("d"),
        ]);
        state.pinned = vec![0, 1, 3];
        let batches = state.read_batches();
        assert_eq!(batches.len(), 1);
        let indexes: Vec<_> = batches[0]
            .iter()
            .map(|read| read.descriptor_index)
            .collect();
        assert_eq!(indexes, vec![0, 1, 3]);
    }

    #[test]
    fn set_descriptors_drops_missing_workspace_vars() {
        let mut state = InspectorState::default();
        state.set_descriptors(vec![descriptor("a"), descriptor("b")]);
        state.pinned = vec![0, 1];
        state.watch_vars.push(WatchEntry {
            var_name: "b".to_owned(),
            descriptor_index: 1,
            write_buf: "1.0".to_owned(),
        });
        state.set_descriptors(vec![descriptor("a")]);
        assert_eq!(state.pinned, vec![0]);
        assert!(state.watch_vars.is_empty());
    }
}
