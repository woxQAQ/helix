# Using Helix

For a full interactive introduction to Helix, refer to the
[tutor](https://github.com/helix-editor/helix/blob/master/runtime/tutor) which
can be accessed via the command `hx --tutor` or `:tutor`.

> 💡 Currently, not all functionality is fully documented, please refer to the
> [key mappings](./keymap.md) list.

## Modes

Helix is a modal editor, meaning it has different modes for different tasks. The main modes are:

* [Normal mode](./keymap.md#normal-mode): For navigation and editing commands. This is the default mode.
* [Insert mode](./keymap.md#insert-mode): For typing text directly into the document. Access by typing `i` in normal mode.
* [Select/extend mode](./keymap.md#select--extend-mode): For making selections and performing operations on them. Access by typing `v` in normal mode.

## Reviewing a repository

Run `hx --comments .` to open the file picker and start a review session. Use normal editing commands, including `gd` for go-to-definition and `gr` for references when the language server supports them.

Press `Space n` in Normal or Select mode to see the review menu. For a new comment, type the text and press `Enter` to add it or `Esc` to cancel.

| Keys | Command | Action |
| --- | --- | --- |
| `Space n c` | `:comment Check the error path` | Comment on the current line or the primary multiline selection. For example, `2x` selects two lines. |
| `Space n p` | `:comments` | Search all session comments. Press `Enter` to jump to the target or `Delete` to remove the highlighted comment. |
| `Space n D` | `:comments-clear` | Delete all comments in the current session, including comments on closed buffers. |
| `Space n y` | `:comments-yank` | Copy the full report to the system clipboard. |

A block comment has a guide beside its source lines, running from the first line to the frame below the last. The frame header shows the inclusive range, such as `L12–L15`. In Normal mode, move through the frame with `j` and `k`, or click a comment row. Press `dd` on the `L…` header to delete that comment. Press `Enter`, `i`, or `Space n c` to edit its text directly inside the frame. Use the arrow keys to move the caret and `Shift-Enter` or `Alt-Enter` to insert a newline. `Enter` saves the draft and `Esc` cancels it. Press `Esc` on a comment row to return to the source cursor. Select mode continues to select source text across comments.

Comment text wraps to the width of each split. The frame uses `ui.virtual.review` and the body uses `ui.text`. Copying and saving the source buffer includes only source text.

Comment text is literal, including spaces, quotes and `%` expressions. Comments belong to the session and survive closing a file buffer. The report uses the current buffer's line numbers, including unsaved edits. For closed buffers it uses their last reviewed text. A fully deleted target stays in the report with `(target removed)` beside its nearest position. Undoing the deletion in the same buffer restores the original anchor.

Helix prints the report to stdout on exit. For example, `hx --comments . | save review.txt` in Nushell saves it for later. Each entry contains the path, a one-based line or inclusive line range when applicable, and the comment text.

## Buffers

Buffers are in-memory representations of files. You can have multiple buffers open at once. Use [pickers](./pickers.md) or commands like `:buffer-next` and `:buffer-previous` to open buffers or switch between them.

## Selection-first editing

Inspired by [Kakoune](http://kakoune.org/), Helix follows the `selection → action` model. This means that whatever you are going to act on (a word, a paragraph, a line, etc.) is selected first and the action itself (delete, change, yank, etc.) comes second. A cursor is simply a single width selection.

## Multiple selections

Also inspired by Kakoune, multiple selections are a core mode of interaction in Helix. For example, the standard way of replacing multiple instances of a word is to first select all instances (so there is one selection per instance) and then use the change action (`c`) to edit them all at the same time.

## Motions

Motions are commands that move the cursor or modify selections. They're used for navigation and text manipulation. Examples include `w` to move to the next word, or `f` to find a character. See the [Movement](./keymap.md#movement) section of the keymap for more motions.

