use std::collections::HashMap;

pub fn slice_from_frames(
    frames: &HashMap<(u8, u32), Vec<u8>>,
    group_id: u8,
    stream_offset: u64,
    stream_raw_size: u64,
    frame_size: u64,
) -> anyhow::Result<Vec<u8>> {
    if stream_raw_size == 0 {
        return Ok(Vec::new());
    }
    let start_frame = (stream_offset / frame_size) as u32;
    let end_frame = ((stream_offset + stream_raw_size - 1) / frame_size) as u32;
    let mut result = Vec::with_capacity(stream_raw_size as usize);
    for fi in start_frame..=end_frame {
        let frame = frames
            .get(&(group_id, fi))
            .ok_or_else(|| anyhow::anyhow!("missing frame ({},{})", group_id, fi))?;
        let frame_start = fi as u64 * frame_size;
        let s = if fi == start_frame {
            (stream_offset - frame_start) as usize
        } else {
            0
        };
        let e = if fi == end_frame {
            ((stream_offset + stream_raw_size) - frame_start) as usize
        } else {
            frame.len()
        };
        result.extend_from_slice(&frame[s..e]);
    }
    Ok(result)
}
