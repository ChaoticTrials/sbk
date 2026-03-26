use crate::format::index::IndexEntry;

pub(crate) enum TreeNode {
    Dir {
        name: String,
        children: Vec<TreeNode>,
    },
    File {
        name: String,
        original_size: u64,
        group_id: u8,
    },
}

pub(crate) fn build(entries: &[IndexEntry]) -> TreeNode {
    let mut root = TreeNode::Dir {
        name: String::new(),
        children: Vec::new(),
    };

    for entry in entries {
        let segments: Vec<&str> = entry.path.split('/').collect();
        insert_entry(&mut root, &segments, entry.original_size, entry.group_id);
    }

    sort_tree(&mut root);
    root
}

fn insert_entry(node: &mut TreeNode, segments: &[&str], original_size: u64, group_id: u8) {
    if segments.is_empty() {
        return;
    }

    let TreeNode::Dir { children, .. } = node else {
        return;
    };

    if segments.len() == 1 {
        children.push(TreeNode::File {
            name: segments[0].to_string(),
            original_size,
            group_id,
        });
    } else {
        // Find or create the intermediate Dir node
        let dir_name = segments[0];
        let existing = children
            .iter_mut()
            .find(|c| matches!(c, TreeNode::Dir { name, .. } if name == dir_name));

        if let Some(dir) = existing {
            insert_entry(dir, &segments[1..], original_size, group_id);
        } else {
            let mut new_dir = TreeNode::Dir {
                name: dir_name.to_string(),
                children: Vec::new(),
            };
            insert_entry(&mut new_dir, &segments[1..], original_size, group_id);
            children.push(new_dir);
        }
    }
}

fn sort_tree(node: &mut TreeNode) {
    if let TreeNode::Dir { children, .. } = node {
        for child in children.iter_mut() {
            sort_tree(child);
        }
        children.sort_by(|a, b| {
            let a_is_dir = matches!(a, TreeNode::Dir { .. });
            let b_is_dir = matches!(b, TreeNode::Dir { .. });
            match (a_is_dir, b_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => {
                    let a_name = match a {
                        TreeNode::Dir { name, .. } | TreeNode::File { name, .. } => name.as_str(),
                    };
                    let b_name = match b {
                        TreeNode::Dir { name, .. } | TreeNode::File { name, .. } => name.as_str(),
                    };
                    a_name.cmp(b_name)
                }
            }
        });
    }
}

pub(crate) fn print(root: &TreeNode, world_name: &str) {
    println!("{}/", world_name);
    if let TreeNode::Dir { children, .. } = root {
        print_children(children, "");
    }
}

fn print_children(children: &[TreeNode], prefix: &str) {
    for (i, node) in children.iter().enumerate() {
        let is_last = i == children.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let extension = if is_last { "    " } else { "│   " };

        match node {
            TreeNode::Dir { name, children } => {
                println!("{}{}{}/", prefix, connector, name);
                print_children(children, &format!("{}{}", prefix, extension));
            }
            TreeNode::File {
                name,
                original_size,
                group_id,
            } => {
                let group_str = group_label(*group_id);
                let size_str = human_size(*original_size);
                println!(
                    "{}{}{:<40} {:<4}  {:>8}",
                    prefix, connector, name, group_str, size_str
                );
            }
        }
    }
}

pub(crate) fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1_048_576 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1_073_741_824 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.2} GB", bytes as f64 / 1_073_741_824.0)
    }
}

fn group_label(id: u8) -> &'static str {
    match id {
        0 => "MCA",
        1 => "NBT",
        2 => "JSON",
        3 => "RAW",
        _ => "???",
    }
}
