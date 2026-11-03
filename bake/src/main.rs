// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

mod test;

fn main() -> bake::Result<()> {
    bake::Registry::discover()?.run()
}

#[path = "bake_generated_tasks/mod.rs"]
mod bake_generated_tasks;
