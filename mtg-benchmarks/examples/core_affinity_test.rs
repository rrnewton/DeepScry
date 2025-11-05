fn main() {
    println!("Testing core_affinity crate...\n");

    // Get available core IDs
    if let Some(core_ids) = core_affinity::get_core_ids() {
        println!("core_affinity::get_core_ids() returned {} cores:", core_ids.len());
        for (i, core_id) in core_ids.iter().enumerate().take(10) {
            println!("  [{}] {:?}", i, core_id);
        }
        if core_ids.len() > 10 {
            println!("  ... and {} more", core_ids.len() - 10);
        }
    } else {
        println!("core_affinity::get_core_ids() returned None!");
    }

    println!("\nSystem CPU info:");
    println!("  num_cpus::get() = {}", num_cpus::get());
    println!("  num_cpus::get_physical() = {}", num_cpus::get_physical());
}
