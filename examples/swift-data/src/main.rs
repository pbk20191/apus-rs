//! SwiftData from Rust. The model is Swift (swift/Todos.swift, built by build.rs); the context, inserts, edits,
//! deletes, saves and fetches are SwiftData's own API, called from Rust.
//! Run: cargo run -p apus-example-swift-data
apus::swift! {
    // The Swift types this program uses, each a Rust type: calls are checked against their declarations.
    class Todo = "Todos.Todo";
    class ModelContainer = "SwiftData.ModelContainer";
    class ModelContext = "SwiftData.ModelContext";
    struct Query = "SwiftData.FetchDescriptor<Todos.Todo>";

    // Declared by name: the rest comes from the modules' symbol graphs (apus-build emits the model module's).
    #[swift_linked = "Todos.todoContainer(path:)"]
    fn todo_container(path: &str) throws -> ModelContainer;
    #[swift_linked = "Todos.allTodos()"]
    fn all_todos() -> Query;
    #[swift_linked = "Todos.openTodos()"]
    fn open_todos() -> Query;

    impl Todo {
        #[swift_linked = "Todos.Todo.init(title:priority:)"]
        fn new(title: &str, priority: i64) -> Todo;
        #[swift_linked = "Todos.Todo.title { get }"]
        fn title(&self) -> String;
        #[swift_linked = "Todos.Todo.priority { get }"]
        fn priority(&self) -> i64;
        #[swift_linked = "Todos.Todo.done { get }"]
        fn done(&self) -> bool;
        #[swift_linked = "Todos.Todo.done { set }"]
        fn set_done(&self, done: bool);
    }

    // SwiftData's, looked up in the framework when first called.
    impl ModelContext {
        #[swift = "SwiftData.ModelContext.init(_:)"]
        fn new(container: &ModelContainer) -> ModelContext;
        #[swift = "SwiftData.ModelContext.insert(_:)", T = "Todos.Todo"]
        fn insert(&self, model: &Todo);
        #[swift = "SwiftData.ModelContext.delete(_:)", T = "Todos.Todo"]
        fn delete(&self, model: &Todo);
        #[swift = "SwiftData.ModelContext.save()"]
        fn save(&self) throws;
        // Generic over the model, which no argument's type gives: bound to `Todo`. The array is read through its
        // `Sequence` conformance.
        #[swift = "SwiftData.ModelContext.fetch(_:)", T = "Todos.Todo"]
        fn fetch(&self, query: &Query) throws -> Vec<Todo>;
    }
}

fn open(path: &str) -> ModelContext {
    let container = todo_container(path).expect("a container");
    ModelContext::new(&container).unwrap()
}

fn show(heading: &str, todos: &[Todo]) {
    println!("{heading}:");
    for todo in todos {
        let mark = if todo.done().unwrap() { "x" } else { " " };
        println!("  [{mark}] {} (priority {})", todo.title().unwrap(), todo.priority().unwrap());
    }
}

fn main() {
    let path = std::env::temp_dir().join(format!("apus-todos-{}.store", std::process::id()));
    let path = path.to_str().unwrap().to_string();

    let context = open(&path);
    for (title, priority) in [("write the report", 3), ("water the plants", 1), ("book flights", 2), ("call the bank", 2)] {
        context.insert(&Todo::new(title, priority).unwrap()).unwrap();
    }
    context.save().expect("save");
    let all = context.fetch(&all_todos().unwrap()).expect("fetch");
    show("all todos, most important first", &all);

    // Edit and delete through the objects fetched, then save.
    for todo in &all {
        match todo.title().unwrap().as_str() {
            "book flights" => todo.set_done(true).unwrap(),
            "water the plants" => context.delete(todo).unwrap(),
            _ => {}
        }
    }
    context.save().expect("save");
    show("not done yet (a #Predicate)", &context.fetch(&open_todos().unwrap()).expect("fetch"));
    drop((all, context));

    // A new container on the same file: what was saved is there.
    let reopened = open(&path);
    show("after reopening the store", &reopened.fetch(&all_todos().unwrap()).expect("fetch"));
    drop(reopened);

    for suffix in ["", "-shm", "-wal"] {
        let _ = std::fs::remove_file(format!("{path}{suffix}"));
    }
}
