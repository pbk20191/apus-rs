//! Builds the Swift model (swift/Todos.swift): the Swift a SwiftData program still needs.
fn main() {
    apus_build::SwiftModule::new("Todos", "swift").build();
}
