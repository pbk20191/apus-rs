//! SwiftData from Rust. The model is Swift (swift/Todos.swift, built by build.rs); the context, inserts, edits,
//! deletes, saves and fetches are SwiftData's own API, called from Rust.
//! Run: cargo run -p apus-example-swift-data
use apus::swift::{DirectValue, SwiftIter, SwiftObject, SwiftValue};

apus::swift! {
    // The model module's own declarations, linked from the static library.
    #[swift_linked = "Todos.todoContainer(path: String) throws -> SwiftData.ModelContainer(class)"]
    fn todo_container(path: &str) throws -> SwiftObject;
    #[swift_linked = "Todos.allTodos() -> SwiftData.FetchDescriptor(struct)<Todo(class)>"]
    fn all_todos() -> SwiftValue;
    #[swift_linked = "Todos.openTodos() -> SwiftData.FetchDescriptor(struct)<Todo(class)>"]
    fn open_todos() -> SwiftValue;
    #[swift_linked = "Todos.Todo(class).init(title: String, priority: Int)"]
    fn new_todo(title: &str, priority: i64) -> SwiftObject;
    #[swift_linked = "Todos.Todo(class).title: String { get }"]
    fn title(self: &SwiftObject) -> String;
    #[swift_linked = "Todos.Todo(class).priority: Int { get }"]
    fn priority(self: &SwiftObject) -> i64;
    #[swift_linked = "Todos.Todo(class).done: Bool { get }"]
    fn done(self: &SwiftObject) -> bool;
    #[swift_linked = "Todos.Todo(class).done: Bool { set }"]
    fn set_done(self: &SwiftObject, done: bool);

    // SwiftData's, looked up in the framework when first called.
    #[swift = "SwiftData.ModelContext(class).init(_: ModelContainer(class))"]
    fn new_context(container: &SwiftObject) -> SwiftObject;
    #[swift = "SwiftData.ModelContext(class).insert<T: PersistentModel(AnyObject)>(_: T)"]
    fn insert(self: &SwiftObject, model: &SwiftObject);
    #[swift = "SwiftData.ModelContext(class).delete<T: PersistentModel(AnyObject)>(_: T)"]
    fn delete(self: &SwiftObject, model: &SwiftObject);
    #[swift = "SwiftData.ModelContext(class).save() throws"]
    fn save(self: &SwiftObject) throws;
    // Generic over the model, which no argument's type gives: bound to `Todo`. The array comes back in registers
    // (`DirectValue`), and `SwiftIter` walks it through its `Sequence` conformance.
    #[swift = "SwiftData.ModelContext(class).fetch<T: PersistentModel(AnyObject)>(_: FetchDescriptor(struct)<T>) \
               throws -> [T]", T = "Todos.Todo(class)"]
    fn fetch_todos(self: &SwiftObject, descriptor: &SwiftValue) throws -> DirectValue;
}

struct Store {
    context: SwiftObject,
}

impl Store {
    fn open(path: &str) -> Store {
        let container = todo_container(path).expect("a container");
        Store { context: new_context(&container).unwrap() }
    }

    fn add(&self, title: &str, priority: i64) {
        insert(&self.context, &new_todo(title, priority).unwrap()).unwrap();
    }

    /// The todos `descriptor` selects.
    fn fetch(&self, descriptor: SwiftValue) -> Vec<SwiftObject> {
        let found = fetch_todos(&self.context, &descriptor).expect("fetch");
        SwiftIter::new(found.0).expect("an array").map(|todo| todo.into_object().expect("a Todo is an object")).collect()
    }

    fn save(&self) {
        save(&self.context).expect("save");
    }
}

fn show(heading: &str, todos: &[SwiftObject]) {
    println!("{heading}:");
    for todo in todos {
        let mark = if done(todo).unwrap() { "x" } else { " " };
        println!("  [{mark}] {} (priority {})", title(todo).unwrap(), priority(todo).unwrap());
    }
}

fn main() {
    let path = std::env::temp_dir().join(format!("apus-todos-{}.store", std::process::id()));
    let path = path.to_str().unwrap().to_string();

    let store = Store::open(&path);
    for (title, priority) in [("write the report", 3), ("water the plants", 1), ("book flights", 2), ("call the bank", 2)] {
        store.add(title, priority);
    }
    store.save();
    let all = store.fetch(all_todos().unwrap());
    show("all todos, most important first", &all);

    // Edit and delete through the objects fetched, then save.
    for todo in &all {
        match title(todo).unwrap().as_str() {
            "book flights" => set_done(todo, true).unwrap(),
            "water the plants" => delete(&store.context, todo).unwrap(),
            _ => {}
        }
    }
    store.save();
    show("not done yet (a #Predicate)", &store.fetch(open_todos().unwrap()));
    drop((all, store));

    // A new container on the same file: what was saved is there.
    let reopened = Store::open(&path);
    show("after reopening the store", &reopened.fetch(all_todos().unwrap()));
    drop(reopened);

    for suffix in ["", "-shm", "-wal"] {
        let _ = std::fs::remove_file(format!("{path}{suffix}"));
    }
}
