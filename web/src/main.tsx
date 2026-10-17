import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Theme } from "@radix-ui/themes";
import "@radix-ui/themes/styles.css";
import "./styles.css";
import { onSessionReset } from "./lib/session";
import { App } from "./app";

const queryClient = new QueryClient({
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
createRoot(document.getElementById("root")!).render(
    <Theme appearance="light" accentColor="gray" grayColor="gray" radius="medium" scaling="100%">
        <QueryClientProvider client={queryClient}>
            <App />
        </QueryClientProvider>
    </Theme>,
);
