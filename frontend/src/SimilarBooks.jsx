import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useEffect, useState, Suspense} from 'react';
import PropTypes from 'prop-types';
import {jsonGetReq} from './Common';

import {Card} from 'react-bootstrap';

export default function SimilarBooks({bookId, onSelect}) {
    const [similarBooks, setSimilarBooks] = useState([]);

    useEffect(() => {
        if (bookId) {
            jsonGetReq(`/similar/${bookId}`, null, (data) => {
                setSimilarBooks(data);
            }, (error) => {
                console.error(error);
            });
        }
    }, [bookId]);

    return (
        <Card>
            <Card.Header>
                유사한 책 목록
            </Card.Header>
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <Card.Body>
                    {similarBooks && similarBooks.length > 0 ? (
                        similarBooks.map((book) => (
                            <div key={book.book_id} style={{cursor: 'pointer', padding: '4px', borderBottom: '1px solid #eee'}} onClick={() => onSelect && onSelect(`${book.category}/${book.book_id}`)}>
                                {book.title} - {book.author}
                            </div>
                        ))
                    ) : (
                        <div>유사한 책이 없습니다.</div>
                    )}
                </Card.Body>
            </Suspense>
        </Card>
    );
}

SimilarBooks.propTypes = {
    bookId: PropTypes.oneOfType([PropTypes.number, PropTypes.string]),
    onSelect: PropTypes.func
};
