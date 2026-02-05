import {useEffect, useState} from 'react';
import {useOutletContext} from 'react-router-dom';

import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';
import {Container, Row, Col} from 'react-bootstrap';

import {jsonGetReq} from './Common';
import CategoryMapping from './CategoryMapping';

export default function Admin() {
    const {searchResults, hasSearched} = useOutletContext();
    const [categoryList, setCategoryList] = useState([]);
    const [errorMessage, setErrorMessage] = useState('');

    useEffect(() => {
        const categoryListUrl = '/categories';
        jsonGetReq(categoryListUrl, null, (categories) => {
            // _root 제외하고 정렬
            const filtered = categories.filter(c => c !== '_root').sort((a, b) => a.localeCompare(b));
            setCategoryList(filtered);
        }, (error) => {
            setErrorMessage(`카테고리 목록을 불러올 수 없습니다. ${error}`);
        });
    }, []);

    return (
        <Container id="edit">
            <Row>
                <Col>
                    <h5 className="mb-3">관리</h5>
                    {errorMessage && (
                        <div className="alert alert-danger">{errorMessage}</div>
                    )}
                    <CategoryMapping categoryList={categoryList}/>
                </Col>
            </Row>
        </Container>
    );
}
