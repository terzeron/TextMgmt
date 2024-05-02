import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useEffect, Suspense} from 'react';

import {Card} from 'react-bootstrap';

export default function SimilarBooks() {
    useEffect(() => {

    }, [] /* rendered once */);

    return (
        <Card>
            <Card.Header>
                유사한 파일 목록
            </Card.Header>
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <Card.Body>
                </Card.Body>
            </Suspense>
        </Card>
    );
}
