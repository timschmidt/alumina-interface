//! Print deterministic exact/CAM checkpoint facts without opening a window.

use std::fmt::Write as _;

use alumina_interface_core::{
    compile_representative_program, package_canonical_program, representative_partition_policy,
};

fn hexadecimal(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn main() {
    let program = compile_representative_program().expect("representative compilation must pass");
    let final_steps = program
        .segments()
        .iter()
        .fold([0_i64; 2], |mut total, segment| {
            total[0] += segment.delta_steps[0];
            total[1] += segment.delta_steps[1];
            total
        });
    let end_tick = program
        .time_boundaries()
        .last()
        .expect("compiled path retains a terminal boundary")
        .tick()
        .get();
    let partition = package_canonical_program(
        &program,
        representative_partition_policy().expect("representative partition policy must pass"),
    )
    .expect("representative partition packaging must pass");

    println!("source_curves={}", program.source().curves().len());
    println!(
        "source_fragments={}",
        program.evidence().source_fragment_count()
    );
    println!("canonical_segments={}", program.segments().len());
    println!("final_steps={},{}", final_steps[0], final_steps[1]);
    println!("end_tick={end_tick}");
    println!(
        "ideal_chord_path_length_mm={}",
        program.ideal_chord_path_length_mm()
    );
    println!(
        "ideal_chord_path_length_mm_display_f64={:.12}",
        program
            .ideal_chord_path_length_mm()
            .to_f64_lossy()
            .expect("fixture length has a finite display projection")
    );
    println!(
        "source_chord_error_mm={}",
        program.evidence().maximum_source_chord_error_mm()
    );
    println!(
        "curve_to_canonical_chord_bound_mm={}",
        program
            .evidence()
            .maximum_curve_to_canonical_chord_error_mm()
    );
    println!(
        "curve_to_canonical_chord_bound_mm_display_f64={:.12}",
        program
            .evidence()
            .maximum_curve_to_canonical_chord_error_mm()
            .to_f64_lossy()
            .expect("fixture bound has a finite display projection")
    );
    println!(
        "timer_boundary_error_seconds={}",
        program.evidence().maximum_timer_boundary_error_seconds()
    );
    println!(
        "segment_duration_error_seconds={}",
        program.evidence().maximum_segment_duration_error_seconds()
    );
    println!("partition_blocks={}", partition.block_count());
    println!("partition_bytes={}", partition.bytes().len());
    println!("storage_chunks={}", partition.chunks().len());
    println!(
        "maximum_observed_block_ticks={}",
        partition.maximum_observed_block_ticks()
    );
    println!(
        "partition_sha256={}",
        hexadecimal(&partition.publication().object.content.digest.0)
    );
    println!(
        "chunk_manifest_sha256={}",
        hexadecimal(&partition.publication().manifest.digest.0)
    );
    println!(
        "terminal_block_sha256={}",
        hexadecimal(&partition.terminal_progress().block_digest.0)
    );
}
