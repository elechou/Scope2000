pub mod panel;

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
    root
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
