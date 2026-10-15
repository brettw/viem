//! # Comment Markdown demo
//!
//! Open this file in Viem. **View > Markdown in comments** is on by default
//! and presents a *small subset* of Markdown inside Code comments: **strong**,
//! _emphasis_, ~~strikethrough~~, [links](https://github.github.com/gfm/) and
//! `code spans`. Comment markers such as `//!`, `///`, `//` and ` * ` keep the
//! comment's own appearance, every delimiter stays visible, and nothing changes
//! the source. `:set nocommentmarkdown` shows the plain comments for comparison.
//!
//! Only character syntax is recognized, so the `#` above is not a heading.
//!
//! ```
//! // A fence without a language uses the document's language (Rust).
//! let answer: u32 = 6 * 7;
//! ```
//!
//! Run the program with `cargo run --example comment_markdown_demo`.

/*!
Inner block documentation is one comment, so this fence needs no closing
backticks: the end of the comment closes it.

```json
{ "language": "json", "closed_by": "the end of this comment" }
*/

// Plain line comments group while their indentation and leader match. A span
// can **continue across
// several lines**: the `//` leaders between its parts are never styled.
//
// A blank comment line ends a paragraph, so this **does not
//
// close** across the gap.
//
// Emphasis: *single asterisks*, _underscores_, **double asterisks**,
// __double underscores__, ***both at once***, and ~~strikethrough~~.
// Intraword underscores such as snake_case_name stay literal, and so do
// escaped \*asterisks\* and Markdown inside code: `**not bold**`.
//
// Links: [the GFM specification](https://github.github.com/gfm/ "GitHub Flavored Markdown"),
// an autolink <https://www.rust-lang.org>, and a bare URL https://doc.rust-lang.org/book/.
//
// Block structure stays literal text:
// # not a heading
// - not a list item
// > not a quotation

/// A point in the plane.
///
/// Outer documentation comments (`///`) group separately from plain `//`
/// comments. Inline code uses the document's language: `Point { x: 1.0, y: 2.0 }`.
/// A code span can also cross lines: `Point::new(
/// 3.0, 4.0)`.
///
/// ```rust
/// let origin = Point::new(0.0, 0.0);
/// assert_eq!(origin.distance(&Point::new(3.0, 4.0)), 5.0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    x: f64,
    y: f64,
}

impl Point {
    /**
     * Creates a point. Starred block comments skip their ` * ` middles, so
     * **strong text can
     * continue** across them.
     *
     * ```python
     * # A fence with a declared language is highlighted as that language.
     * def new(x, y):
     *     return {"x": x, "y": y}
     * ```
     */
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// The Euclidean distance to `other`. This fence has no closing backticks,
    /// so the end of the comment closes it:
    ///
    /// ```sh
    /// cargo run --example comment_markdown_demo
    fn distance(&self, other: &Point) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

/* A one-line block comment with _emphasis_ and `code`. */

/*
   Unstarred block comments keep their indentation as margin; *emphasis*,
   **strong** and [links](https://example.com) still work.
*/

/* Rust block comments nest: **outer /* the nested comment keeps its own
   *plain* text */ and the outer comment** resumes after it. */

fn main() {
    // Comment-like text inside strings is code, never a comment:
    let url = "https://example.com // **not a comment**";
    let pattern = "/* **also not a comment** */";
    let origin = Point::new(0.0, 0.0);
    let point = Point::new(3.0, 4.0);
    let distance = origin.distance(&point); // **Trailing** comments stand alone: `distance`.
    assert_eq!(distance, 5.0);
    println!("{url}\n{pattern}\n{origin:?} to {point:?}: {distance}");
}
