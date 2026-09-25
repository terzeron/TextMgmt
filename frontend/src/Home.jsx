import { useEffect } from "react";
import { useOutletContext } from "react-router-dom";

// vite define이 박는 빌드 ID(__APP_BUILD_ID__)를 보여 각 브라우저가
// 실제로 최신 배포를 받았는지 확인할 수 있게 한다.
/* global __APP_BUILD_ID__ */
const BUILD_ID = typeof __APP_BUILD_ID__ !== "undefined" ? __APP_BUILD_ID__ : "dev";

export default function Home() {
    const { role } = useOutletContext();

    useEffect(() => {
        if (role) {
            fetch('/wake').catch(() => {});
        }
    }, [role]);

    return (
        <div>
            <div className="jumbotron jumbotron-fluid">
                <div className="container mt-3 ms-3">
                    <h1 className="display-5">우진은채네 책방</h1>
                    <p className="lead">책과 만화가 함께하는 책방에 오신 것을 환영합니다.</p>
                </div>
            </div>
            <footer
                data-testid="build-id"
                className="text-muted text-center"
                style={{ fontSize: "12px", padding: "8px" }}
            >
                build: {BUILD_ID}
            </footer>
        </div>
    );
}
