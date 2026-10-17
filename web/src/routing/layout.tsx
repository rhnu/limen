import { useEffect, useState } from "react";
import { Link, NavLink, Outlet, useLocation, useNavigate, useNavigation } from "react-router";
import { Button } from "@radix-ui/themes";
import { LogOut } from "lucide-react";
import { replaceSession, useSession } from "../session/store";
import { restoreSession } from "../session/restore";
import { Failure, Loading } from "../components/common";
import { Login } from "../components/login";
import { managementNavigation, paths, safeReturnTo } from "./paths";

export function RootLayout() {
    const storageError = useSession((state) => state.storageError);
    useEffect(() => {
        void restoreSession();
    }, []);
    return (
        <>
            {storageError && (
                <div className="global-notice" role="alert">
                    {storageError}
                </div>
            )}
            <Outlet />
        </>
    );
}

function LoginGate() {
    const location = useLocation();
    const navigate = useNavigate();
    const [open, setOpen] = useState(true);
    return (
        <>
            <h1>管理登录</h1>
            <Button variant="outline" onClick={() => setOpen(true)}>
                登录
            </Button>
            <Login
                gate
                open={open}
                destination={safeReturnTo(location.pathname + location.search)}
                onOpenChange={(value) => {
                    setOpen(value);
                    if (!value) void navigate(paths.home, { replace: true });
                }}
            />
        </>
    );
}

export function SessionGate() {
    const phase = useSession((state) => state.phase);
    const verificationError = useSession((state) => state.verificationError);
    if (phase === "authenticated") return <AdminLayout />;
    return (
        <main className="gate">
            <Link className="wordmark" to={paths.home}>
                LIMEN
            </Link>
            {phase === "verifying" ? (
                <Loading />
            ) : phase === "blocked" ? (
                <>
                    <h1>管理会话</h1>
                    <Failure
                        error={verificationError}
                        reset={() => {
                            useSession.setState({ phase: "verifying" });
                            void restoreSession();
                        }}
                    />
                    <Button variant="outline" mt="4" onClick={() => replaceSession(null, "public")}>
                        更换密钥
                    </Button>
                </>
            ) : (
                <LoginGate />
            )}
        </main>
    );
}

function AdminLayout() {
    const navigate = useNavigate();
    const navigation = useNavigation();
    return (
        <>
            <header className="admin-header">
                <div className="admin-header-inner">
                    <Link className="wordmark" to={paths.home}>
                        LIMEN
                    </Link>
                    <nav aria-label="管理导航">
                        {managementNavigation.map(({ to, label }) => (
                            <NavLink
                                key={to}
                                to={to}
                                className={({ isActive }) => (isActive ? "active" : "")}
                            >
                                {label}
                            </NavLink>
                        ))}
                    </nav>
                    <Button
                        className="logout"
                        variant="ghost"
                        onClick={() => {
                            replaceSession(null, "public");
                            void navigate(paths.home);
                        }}
                    >
                        <LogOut size={15} aria-hidden="true" />
                        退出
                    </Button>
                </div>
            </header>
            <main className="admin-main">
                {navigation.state === "idle" ? <Outlet /> : <Loading />}
            </main>
        </>
    );
}

export function NotFound() {
    return (
        <main className="gate">
            <h1>页面不存在</h1>
            <Link to={paths.home}>返回首页</Link>
        </main>
    );
}

/** Render controlled recovery text without exposing route, chunk, or component exceptions. */
export function RouteFailure() {
    return (
        <main className="gate" role="alert">
            <h1>页面加载失败</h1>
            <p>请重新加载页面后再试。</p>
            <Button variant="outline" onClick={() => window.location.reload()}>
                重新加载页面
            </Button>
            <Link to={paths.home}>返回首页</Link>
        </main>
    );
}
