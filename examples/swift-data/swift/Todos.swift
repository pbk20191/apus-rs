// The Swift a Rust program using SwiftData still needs, and nothing more: `@Model` is a macro that writes the
// `PersistentModel` conformance; a container's schema takes `any PersistentModel.Type` values; sort descriptors take
// key paths and `#Predicate` is a macro, both made only by the compiler. Everything else is called from Rust.
import Foundation
import SwiftData

@Model public final class Todo {
    public var title: String
    public var priority: Int
    public var done: Bool
    public init(title: String, priority: Int) {
        self.title = title
        self.priority = priority
        self.done = false
    }
}

/// A container for `Todo` whose store is the file at `path`, created if missing.
public func todoContainer(path: String) throws -> ModelContainer {
    try ModelContainer(for: Todo.self, configurations: ModelConfiguration(url: URL(fileURLWithPath: path)))
}

/// Every todo, the most important first.
public func allTodos() -> FetchDescriptor<Todo> {
    FetchDescriptor(sortBy: [SortDescriptor(\.priority, order: .reverse), SortDescriptor(\.title)])
}

/// The todos not done yet, by title.
public func openTodos() -> FetchDescriptor<Todo> {
    FetchDescriptor(predicate: #Predicate { !$0.done }, sortBy: [SortDescriptor(\.title)])
}
