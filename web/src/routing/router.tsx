import { createHashRouter } from "react-router";
import { Home } from "../pages/home";
import { Loading } from "../components/common";
import { NotFound, RootLayout, RouteFailure, SessionGate } from "./layout";
import { paths } from "./paths";

// Data fetching stays behind SessionGate in React Query; route discovery only loads code.
export const router = createHashRouter([
    {
        Component: RootLayout,
        ErrorBoundary: RouteFailure,
        HydrateFallback: Loading,
        children: [
            { path: paths.home, Component: Home },
            {
                Component: SessionGate,
                children: [
                    {
                        path: paths.usage,
                        lazy: async () => ({ Component: (await import("../pages/usage")).Usage }),
                    },
                    {
                        path: paths.requests,
                        lazy: async () => ({
                            Component: (await import("../pages/requests")).Requests,
                        }),
                    },
                    {
                        path: paths.requestPattern,
                        lazy: async () => ({
                            Component: (await import("../pages/requests")).RequestDetail,
                        }),
                    },
                    {
                        path: paths.logs,
                        lazy: async () => ({ Component: (await import("../pages/logs")).Logs }),
                    },
                    {
                        path: paths.status,
                        lazy: async () => ({ Component: (await import("../pages/status")).Status }),
                    },
                    {
                        path: paths.config,
                        lazy: async () => ({ Component: (await import("../pages/config")).Config }),
                    },
                ],
            },
            { path: "*", Component: NotFound },
        ],
    },
]);
