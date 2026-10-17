export type CursorState = { identity: string; cursors: (string | undefined)[] };
export type CursorAction =
    | { type: "next"; cursor: string | null | undefined }
    | { type: "previous" }
    | { type: "reset" };

export function initialCursor(identity: string): CursorState {
    return { identity, cursors: [undefined] };
}

/** Keep only explicitly visited windows, with a fixed upper bound per query. */
export function moveCursor(
    state: CursorState,
    identity: string,
    action: CursorAction,
): CursorState {
    const current = state.identity === identity ? state : initialCursor(identity);
    switch (action.type) {
        case "next":
            return action.cursor && current.cursors.length < 100
                ? { identity, cursors: [...current.cursors, action.cursor] }
                : current;
        case "previous":
            return current.cursors.length > 1
                ? { identity, cursors: current.cursors.slice(0, -1) }
                : current;
        case "reset":
            return initialCursor(identity);
    }
}
