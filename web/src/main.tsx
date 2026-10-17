import { createRoot } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { Theme } from "@radix-ui/themes";
import "@radix-ui/themes/styles.css";
import "./styles.css";
import { queryClient } from "./api/query-client";
import { App } from "./app";

const root = document.getElementById("root");
if (!root) throw new Error("Missing application root");
createRoot(root).render(
    <Theme appearance="light" accentColor="gray" grayColor="gray" radius="medium" scaling="100%">
        <QueryClientProvider client={queryClient}>
            <App />
        </QueryClientProvider>
    </Theme>,
);
