use std::fs::File;
use std::io::Seek;
use std::io::SeekFrom;
use std::path::Path;

use crate::classify::Group;
use crate::format::frame_dir::read_frame_dir;
use crate::format::header::read_header;
use crate::format::index::read_index;

pub fn info(archive: &Path, list: bool) -> anyhow::Result<()> {
    let mut f = File::open(archive)?;
    let header = read_header(&mut f)?;

    f.seek(SeekFrom::Start(header.frame_dir_offset))?;
    let frame_dir = read_frame_dir(&mut f)?;

    f.seek(SeekFrom::Start(header.index_offset))?;
    let entries = read_index(&mut f, header.index_compressed_size, header.index_checksum)?;

    println!("SBK Archive: {}", archive.display());
    println!("  Format version: {}", header.format_version);
    println!("  Files stored:   {}", header.file_count);
    println!(
        "  Frame size:     {} MiB",
        header.frame_size_bytes / (1024 * 1024)
    );

    // Count by group
    let mut counts = [0u64; 4];
    for e in &entries {
        if let Some(g) = Group::from_u8(e.group_id) {
            counts[g as u8 as usize] += 1;
        }
    }
    println!("  MCA files:  {}", counts[0]);
    println!("  NBT files:  {}", counts[1]);
    println!("  JSON files: {}", counts[2]);
    println!("  RAW files:  {}", counts[3]);

    // Frame directory summary
    println!("Frame directory:");
    let group_names = ["MCA", "NBT", "JSON", "RAW"];
    for (g, name) in group_names.iter().enumerate() {
        let frames = &frame_dir.groups[g];
        let total_compressed: u64 = frames.iter().map(|f| f.frame_compressed_sz as u64).sum();
        println!(
            "  Group {g} ({name}): {} frame(s), {} bytes compressed",
            frames.len(),
            total_compressed
        );
    }

    if list {
        println!("\nFile manifest:");
        println!(
            "{:<60} {:>12} {:>5} {:>20}",
            "Path", "Original", "Grp", "mtime_ms"
        );
        println!("{}", "-".repeat(100));
        for e in &entries {
            println!(
                "{:<60} {:>12} {:>5} {:>20}",
                e.path, e.original_size, e.group_id, e.mtime_ms,
            );
        }
        println!("\nNote: files excluded during compression are absent from this archive.");
    }

    Ok(())
}
