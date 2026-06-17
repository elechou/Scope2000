pub mod panel;

use crate::source::{ParamWrite, VarDescriptor};

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

        self.entries = build_descriptor_tree(&descriptors);
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

    pub fn update_values(&mut self, start: u16, values: Vec<u32>) {
        let start = usize::from(start);
        for (offset, bits) in values.into_iter().enumerate() {
            let index = start + offset;
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

    pub fn read_ranges(&self) -> Vec<(u16, u8)> {
        let indexes = self.watched_indexes();
        if indexes.is_empty() {
            return Vec::new();
        }

        let mut ranges = Vec::new();
        let mut start = indexes[0];
        let mut prev = indexes[0];
        for index in indexes.into_iter().skip(1) {
            let count = prev - start + 1;
            if index == prev + 1 && count < 32 {
                prev = index;
            } else {
                ranges.push((start as u16, count as u8));
                start = index;
                prev = index;
            }
        }
        ranges.push((start as u16, (prev - start + 1) as u8));
        ranges
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

pub fn build_descriptor_tree(descriptors: &[VarDescriptor]) -> Vec<DescriptorEntry> {
    let mut root = Vec::<DescriptorEntry>::new();
    for (index, descriptor) in descriptors.iter().enumerate() {
        insert_descriptor(&mut root, &descriptor.name, index);
    }
    root
}

fn insert_descriptor(entries: &mut Vec<DescriptorEntry>, name: &str, index: usize) {
    let parts: Vec<&str> = name.split('.').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        entries.push(DescriptorEntry::Var {
            label: name.to_owned(),
            full_name: name.to_owned(),
            index,
        });
        return;
    }
    insert_parts(entries, &parts, String::new(), index);
}

fn insert_parts(entries: &mut Vec<DescriptorEntry>, parts: &[&str], prefix: String, index: usize) {
    let label = parts[0].to_owned();
    let full_name = if prefix.is_empty() {
        label.clone()
    } else {
        format!("{prefix}.{label}")
    };

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
        insert_parts(members, &parts[1..], full_name, index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{VarRef, VarType};

    fn descriptor(name: &str) -> VarDescriptor {
        VarDescriptor {
            name: name.to_owned(),
            var: VarRef {
                addr: 0,
                ty: VarType::F32,
            },
            kind: 0,
            prescaler: 1,
            group: 0,
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
    fn read_ranges_coalesce_contiguous_indexes() {
        let mut state = InspectorState::default();
        state.set_descriptors(vec![
            descriptor("a"),
            descriptor("b"),
            descriptor("c"),
            descriptor("d"),
        ]);
        state.pinned = vec![0, 1, 3];
        assert_eq!(state.read_ranges(), vec![(0, 2), (3, 1)]);
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
