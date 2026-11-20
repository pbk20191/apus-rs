//! SwiftData for real: the model is Swift (`@Model`, tests/support/notes.swift); the context, inserts, saves and
//! fetches are called from Rust.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{SwiftFn, SwiftObject, SwiftType, SwiftValue};
use common::{check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Notes.noteContainer() throws -> SwiftData.ModelContainer(class)"]
    fn note_container() throws -> SwiftObject;
    #[swift_linked = "Notes.notesByTitle() -> SwiftData.FetchDescriptor(struct)<Note(class)>"]
    fn notes_by_title() -> SwiftValue;
    #[swift_linked = "Notes.Note(class).init(title: String)"]
    fn new_note(self: &SwiftType, title: &str) -> SwiftObject;
    // By name: `title` is a `@Model` property, `{ get set }` in Notes's symbol graph.
    #[swift_linked = "Notes.Note.title { get }"]
    fn title(self: &SwiftObject) -> String;

    #[swift = "SwiftData.ModelContext(class).init(_: ModelContainer(class))"]
    fn new_context(self: &SwiftType, container: &SwiftObject) -> SwiftObject;
    #[swift = "SwiftData.ModelContext(class).insert<T: PersistentModel(AnyObject)>(_: T)"]
    fn insert(self: &SwiftObject, model: &SwiftObject);
    #[swift = "SwiftData.ModelContext(class).save() throws"]
    fn save(self: &SwiftObject) throws;
    #[swift = "SwiftData.FetchDescriptor(struct)<T: PersistentModel(AnyObject)>.includePendingChanges: Bool { set }"]
    fn set_include_pending_changes(self: &mut SwiftValue, include: bool);
    #[swift = "SwiftData.FetchResultsCollection(struct)<Element>.endIndex: Int { get }"]
    fn count(self: &SwiftValue) -> i64;
    #[swift = "SwiftData.FetchResultsCollection(struct)<Element>.subscript(_: Int) -> Element { get }"]
    fn element(self: &SwiftValue, index: i64) -> SwiftValue;
}

/// `fetch<T>(_: FetchDescriptor<T>, batchSize: Int) throws -> FetchResultsCollection<T>`: `swift!` cannot name a
/// result built from generic arguments passed explicitly, so the call gives it. Called through its dispatch thunk, as
/// SwiftData exports it. (`fetch(_:) -> [T]` returns an array, whose elements apus cannot reach: `Array`'s members take
/// it directly, and `Collection`'s subscript is a coroutine.)
static FETCH: SwiftFn = SwiftFn::new(concat!(
    apus::mangle!(
        "SwiftData.ModelContext(class).fetch<T: PersistentModel(AnyObject)>(_: FetchDescriptor(struct)<T>, batchSize: Int) \
         throws -> FetchResultsCollection(struct)<T>"
    ),
    "Tj"
));

static PERSISTENT_MODEL: apus::swift::SwiftProtocol = apus::swift::SwiftProtocol::new("$s9SwiftData15PersistentModelMp");

fn notes_are_inserted_saved_and_fetched() -> Result<(), Failed> {
    let note = SwiftType::from_mangled_name("5Notes4NoteC").ok_or("Notes.Note")?;
    let context_type = SwiftType::from_mangled_name("9SwiftData12ModelContextC").ok_or("SwiftData.ModelContext")?;
    let context = new_context(&context_type, &note_container().map_err(|e| format!("container: {e:?}"))?).unwrap();
    for name in ["milk", "bread", "eggs"] {
        insert(&context, &new_note(&note, name).unwrap()).unwrap();
    }
    save(&context).map_err(|e| format!("save: {e:?}"))?;
    let model = note.conformance(&PERSISTENT_MODEL).ok_or("Note: PersistentModel")?;
    let notes_type = SwiftType::from_mangled_name("9SwiftData22FetchResultsCollectionVy5Notes4NoteCG").ok_or("FetchResultsCollection<Note>")?;
    // Fetching in batches leaves out unsaved changes; everything is saved here.
    let mut by_title = notes_by_title().unwrap();
    set_include_pending_changes(&mut by_title, false).unwrap();
    let notes = FETCH
        .call_returning(notes_type, (&by_title, 2i64, note, model))
        .on(&context)
        .throws()
        .invoke()
        .map_err(|e| format!("fetch: {e:?}"))?;
    let titles: Vec<String> = (0..count(&notes).unwrap())
        .map(|i| title(&element(&notes, i).unwrap().into_object().expect("a Note is a class instance")).unwrap())
        .collect();
    check(titles == ["bread", "eggs", "milk"], format!("{titles:?}"))
}

fn main() {
    common::run(vec![Trial::test("notes_are_inserted_saved_and_fetched", notes_are_inserted_saved_and_fetched)]);
}
