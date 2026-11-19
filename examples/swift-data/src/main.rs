//! SwiftData from Rust. The model is Swift (swift/Todos.swift, built by build.rs); the context, inserts, edits,
//! deletes, saves and fetches are SwiftData's own API, called from Rust.
//! Run: cargo run -p apus-example-swift-data
use apus::swift::{SwiftFn, SwiftObject, SwiftProtocol, SwiftType, SwiftValue, WitnessTable};

apus::swift! {
    // The model module's own declarations, linked from the static library.
    #[swift_linked = "Todos.todoContainer(path: String) throws -> SwiftData.ModelContainer(class)"]
    fn todo_container(path: &str) throws -> SwiftObject;
    #[swift_linked = "Todos.allTodos() -> SwiftData.FetchDescriptor(struct)<Todo(class)>"]
    fn all_todos() -> SwiftValue;
    #[swift_linked = "Todos.openTodos() -> SwiftData.FetchDescriptor(struct)<Todo(class)>"]
    fn open_todos() -> SwiftValue;
    #[swift_linked = "Todos.Todo(class).init(title: String, priority: Int)"]
    fn new_todo(self: &SwiftType, title: &str, priority: i64) -> SwiftObject;
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
    fn new_context(self: &SwiftType, container: &SwiftObject) -> SwiftObject;
    #[swift = "SwiftData.ModelContext(class).insert<T: PersistentModel(AnyObject)>(_: T)"]
    fn insert(self: &SwiftObject, model: &SwiftObject);
    #[swift = "SwiftData.ModelContext(class).delete<T: PersistentModel(AnyObject)>(_: T)"]
    fn delete(self: &SwiftObject, model: &SwiftObject);
    #[swift = "SwiftData.ModelContext(class).save() throws"]
    fn save(self: &SwiftObject) throws;
    #[swift = "SwiftData.FetchDescriptor(struct)<T: PersistentModel(AnyObject)>.includePendingChanges: Bool { set }"]
    fn set_include_pending_changes(self: &mut SwiftValue, include: bool);
    #[swift = "SwiftData.FetchResultsCollection(struct)<Element>.endIndex: Int { get }"]
    fn count(self: &SwiftValue) -> i64;
    #[swift = "SwiftData.FetchResultsCollection(struct)<Element>.subscript(_: Int) -> Element { get }"]
    fn element(self: &SwiftValue, index: i64) -> SwiftValue;
}

/// `fetch(_:batchSize:)`, through the dispatch thunk SwiftData exports. Its result type is built from the generic
/// argument passed explicitly, which the call is given (`call_returning`). (`fetch(_:) -> [T]` returns an `Array`,
/// whose elements apus cannot reach.)
static FETCH: SwiftFn = SwiftFn::new(concat!(
    apus::mangle!(
        "SwiftData.ModelContext(class).fetch<T: PersistentModel(AnyObject)>(_: FetchDescriptor(struct)<T>, batchSize: Int) \
         throws -> FetchResultsCollection(struct)<T>"
    ),
    "Tj"
));

static PERSISTENT_MODEL: SwiftProtocol = SwiftProtocol::new("$s9SwiftData15PersistentModelMp");

struct Store {
    context: SwiftObject,
    todo: SwiftType,
    model: WitnessTable,
    results: SwiftType,
}

impl Store {
    fn open(path: &str) -> Store {
        let todo = SwiftType::from_mangled_name("5Todos4TodoC").expect("Todos.Todo");
        let context_type = SwiftType::from_mangled_name("9SwiftData12ModelContextC").expect("SwiftData.ModelContext");
        let container = todo_container(path).expect("a container");
        Store {
            context: new_context(&context_type, &container).unwrap(),
            todo,
            model: todo.conformance(&PERSISTENT_MODEL).expect("Todo: PersistentModel"),
            results: SwiftType::from_mangled_name("9SwiftData22FetchResultsCollectionVy5Todos4TodoCG").expect("FetchResultsCollection<Todo>"),
        }
    }

    fn add(&self, title: &str, priority: i64) {
        insert(&self.context, &new_todo(&self.todo, title, priority).unwrap()).unwrap();
    }

    /// The todos `descriptor` selects. Fetching in batches leaves out unsaved changes, so save first.
    fn fetch(&self, mut descriptor: SwiftValue) -> Vec<SwiftObject> {
        set_include_pending_changes(&mut descriptor, false).unwrap();
        let found = FETCH
            .call_returning(self.results, (&descriptor, 16i64, self.todo, self.model))
            .on(&self.context)
            .throws()
            .invoke()
            .expect("fetch");
        (0..count(&found).unwrap()).map(|i| element(&found, i).unwrap().into_object().expect("a Todo is an object")).collect()
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
