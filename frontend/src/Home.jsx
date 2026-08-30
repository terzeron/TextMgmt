import { useEffect } from "react";
import { useOutletContext } from "react-router-dom";

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
                    <h1 className="display-5">은채네 책방</h1>
                    <p className="lead">책과 만화가 함께하는 책방에 오신 것을 환영합니다.</p>
                </div>
            </div>
        </div>
    );
}
