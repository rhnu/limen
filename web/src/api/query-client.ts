import { QueryClient } from "@tanstack/react-query";
import { onSessionReset } from "../session/store";

export const queryClient = new QueryClient({
    defaultOptions: {
        queries: {
            retry: false,
            refetchOnWindowFocus: false,
            refetchOnReconnect: false,
            refetchOnMount: false,
            staleTime: Infinity,
            gcTime: 0,
        },
    },
});

onSessionReset(() => {
    void queryClient.cancelQueries();
    queryClient.clear();
});
