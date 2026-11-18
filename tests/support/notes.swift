// A SwiftData model for tests/swift_data.rs: the Swift a Rust program using SwiftData still needs. `@Model` is a
// macro that writes the `PersistentModel` conformance, and a container's schema takes `any PersistentModel.Type`
// values, whose lowering the ABI does not document; everything else is called from Rust.
import Foundation
import SwiftData

@Model public final class Note {
    public var title: String
    public init(title: String) { self.title = title }
}

/// An in-memory container for `Note`.
public func noteContainer() throws -> ModelContainer {
    try ModelContainer(for: Note.self, configurations: ModelConfiguration(isStoredInMemoryOnly: true))
}

/// Notes by title: a sort descriptor takes a key path, which only the compiler makes.
public func notesByTitle() -> FetchDescriptor<Note> {
    FetchDescriptor(sortBy: [SortDescriptor(\.title)])
}
