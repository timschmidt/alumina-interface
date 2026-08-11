//! Print deterministic exact/CAM checkpoint facts without opening a window.

use alumina_interface_core::compile_representative_program;

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
}
