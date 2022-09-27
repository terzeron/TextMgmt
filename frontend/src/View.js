import './View.css';
import {Button, Col, Container, Row} from "react-bootstrap";
import {useState, useEffect} from "react";
import getUrlPrefix from "./Common";
import DirList from "./DirList";


export default function View() {
    const [data, setData] = useState(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState(null);

    function fileClicked(e) {
        e.preventDefault();
    }

    useEffect(() => {
        const getData = async () => {
            try {
                const response = await fetch(getUrlPrefix() + "/dirs/");
                if (!response.ok) {
                    throw new Error(`This is an HTTP error: The status is ${response.status}`);
                }
                let actualData = await response.json();
                const converted = actualData["result"].map((entry) => {
                    if ("items" in entry) {
                        // directory
                        entry["items"].map((subEntry) => {
                            if (!("items" in subEntry)) {
                                subEntry["onClick"] = fileClicked;
                            }
                            return subEntry;
                        });
                    } else {
                        // file
                        entry["onClick"] = fileClicked;
                    }
                    return entry;
                });
                setData(converted);
                setError(null);
            } catch (err) {
                setError(err.message);
                setData(null);
            } finally {
                setLoading(false);
            }
        }
        getData();
        return () => {
            console.log("컴퍼넌트가 사라질 때 cleanup할 일을 여기서 처리해야 함");
        };
    }, [])

    if (loading) return <div>로딩중..</div>;
    if (error) return <div>에러가 발생했습니다</div>;
    if (!data) return null;
    return (
        <Container id="view">
            <Row fluid="true">
                <Col sm={3} className="ps-0 pe-0">
                    <DirList data={data}/>
                </Col>

                <Col sm={9}>
                    <span>
                        <a href="/view/{aaa}" target="_blank">
                            <Button variant="outline-dark">보기</Button>
                        </a>
                        <a href="/view/download/{aaa}" target="_blank">
                            <Button variant="outline-dark">다운로드</Button>
                        </a>
                    </span>
                </Col>
            </Row>
        </Container>
    );
}