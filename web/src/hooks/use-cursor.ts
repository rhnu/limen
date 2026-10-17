import { useState } from "react";
import { initialCursor, moveCursor, type CursorAction } from "../lib/pagination";

export function useCursor(identity: string) {
    const [state, setState] = useState(() => initialCursor(identity));
    // Reset during the identity change so no render issues a request with an old cursor.
    if (state.identity !== identity) setState(initialCursor(identity));
    const current = state.identity === identity ? state : initialCursor(identity);
    function dispatch(action: CursorAction) {
        setState((previous) => moveCursor(previous, identity, action));
    }
    return {
        cursor: current.cursors.at(-1),
        page: current.cursors.length - 1,
        next: (cursor: string | null | undefined) => dispatch({ type: "next", cursor }),
        previous: () => dispatch({ type: "previous" }),
        reset: () => dispatch({ type: "reset" }),
    };
}
