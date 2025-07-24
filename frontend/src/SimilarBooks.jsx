import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useEffect, useState, Suspense} from 'react';
import PropTypes from 'prop-types';

import {Card, ListGroup} from 'react-bootstrap';
import {jsonGetReq} from './Common';

/* eslint-disable react/prop-types */
export default function SimilarBooks({bookId, entryClicked}) {
    const [results, setResults] = useState([]);
    const [isLoading, setIsLoading] = useState(false);
    const [error, setError] = useState(null);

    // Fetch similar books whenever the selected book changes
    useEffect(() => {
        if (!bookId) {
            setResults([]);
            return;
        }

        setIsLoading(true);
        setError(null);

        const url = `/similar/${bookId}`;
        jsonGetReq(url, null, (data) => {
            setResults(data);
            setIsLoading(false);
        }, (err) => {
            setError(err);
            setIsLoading(false);
        });
    }, [bookId]);

    return (
        <Card>
            <Card.Header>
                유사한 파일 목록
            </Card.Header>
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <Card.Body>
                    {isLoading && <p>검색 중...</p>}
                    {error && <p>오류: {error.message}</p>}
                    {!isLoading && !error && results && results.length === 0 && <p>유사한 파일이 없습니다.</p>}
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

SimilarBooks.propTypes = {
    bookId: PropTypes.number,
    entryClicked: PropTypes.func
}
