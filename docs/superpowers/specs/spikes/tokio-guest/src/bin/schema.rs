//! How close do rmcp's macro-generated schemas come to spec/tools.json?
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::{schemars, tool, tool_router};

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Button { Left, Right, Middle }
fn left() -> Button { Button::Left }
fn one() -> i64 { 1 }
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct ClickArgs {
    loc: Vec<i64>,
    #[serde(default = "left")]
    button: Button,
    #[serde(default = "one")]
    clicks: i64,
}
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Direction { Up, Down, Left, Right }
fn down() -> Direction { Direction::Down }
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Axis { Horizontal, Vertical }
fn vertical() -> Axis { Axis::Vertical }
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct ScrollArgs {
    #[serde(default)]
    loc: Option<Vec<i64>>,
    #[serde(default = "vertical", rename = "type")]
    kind: Axis,
    #[serde(default = "down")]
    direction: Direction,
    #[serde(default = "one")]
    wheel_times: i64,
}
struct D;
#[tool_router]
impl D {
    #[tool(name = "Click", description = "c", annotations(title = "Click", read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = false))]
    fn click(&self, Parameters(_a): Parameters<ClickArgs>) -> String { String::new() }
    #[tool(name = "Scroll", description = "s", annotations(title = "Scroll", read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    fn scroll(&self, Parameters(_a): Parameters<ScrollArgs>) -> String { String::new() }
}
fn main() {
    let router: ToolRouter<D> = D::tool_router();
    println!("{}", serde_json::to_string(&router.list_all()).unwrap());
}
