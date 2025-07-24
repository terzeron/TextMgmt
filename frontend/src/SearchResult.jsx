import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Suspense} from 'react';
import {Card, ListGroup} from 'react-bootstrap';

/* eslint-disable react/prop-types */
export default function SearchResult({results, isLoading, error, entryClicked}) {
    return (
        <Card>
            <Card.Header>
                검색 결과
            </Card.Header>
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <Card.Body>
                    {isLoading && <p>검색 중...</p>}
                    {error && <p>오류: {error.message}</p>}
                    {!isLoading && !error && results && results.length === 0 && <p>검색 결과가 없습니다.</p>}
                    {!isLoading && !error && results && results.length > 0 && (
                        <ListGroup>
                            {results.map((result) => (
                                <ListGroup.Item key={result.book_id} action onClick={() => entryClicked(result)}>
                                    {result.title} by {result.author}
                                </ListGroup.Item>
                            ))}
                        </ListGroup>
                    )}
                </Card.Body>
            </Suspense>
        </Card>
    );
}
