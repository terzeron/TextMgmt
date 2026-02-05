import {useEffect, useState, useCallback} from 'react';
import PropTypes from 'prop-types';

import 'bootstrap/dist/css/bootstrap.min.css';
import {Button, Card, Form, InputGroup, ListGroup, Badge, Row, Col} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faPlus, faTrash, faSave} from '@fortawesome/free-solid-svg-icons';

const STORAGE_KEY = 'categoryMappings';

// 카테고리 매핑 데이터 로드
export const loadCategoryMappings = () => {
    try {
        const stored = localStorage.getItem(STORAGE_KEY);
        return stored ? JSON.parse(stored) : {};
    } catch (e) {
        console.error('Failed to load category mappings:', e);
        return {};
    }
};

// 카테고리 매핑 데이터 저장
export const saveCategoryMappings = (mappings) => {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(mappings));
        return true;
    } catch (e) {
        console.error('Failed to save category mappings:', e);
        return false;
    }
};

export default function CategoryMapping({categoryList}) {
    const [mappings, setMappings] = useState({});
    const [selectedCategory, setSelectedCategory] = useState('');
    const [newKeyword, setNewKeyword] = useState('');
    const [message, setMessage] = useState('');

    // 초기 로드
    useEffect(() => {
        const loaded = loadCategoryMappings();
        setMappings(loaded);
    }, []);

    // 카테고리 선택
    const handleCategorySelect = useCallback((category) => {
        setSelectedCategory(category);
        setNewKeyword('');
    }, []);

    // 키워드 추가
    const handleAddKeyword = useCallback(() => {
        if (!selectedCategory || !newKeyword.trim()) return;

        setMappings(prev => {
            const updated = {...prev};
            if (!updated[selectedCategory]) {
                updated[selectedCategory] = [];
            }
            if (!updated[selectedCategory].includes(newKeyword.trim())) {
                updated[selectedCategory] = [...updated[selectedCategory], newKeyword.trim()];
            }
            return updated;
        });
        setNewKeyword('');
    }, [selectedCategory, newKeyword]);

    // 키워드 삭제
    const handleRemoveKeyword = useCallback((keyword) => {
        if (!selectedCategory) return;

        setMappings(prev => {
            const updated = {...prev};
            if (updated[selectedCategory]) {
                updated[selectedCategory] = updated[selectedCategory].filter(k => k !== keyword);
            }
            return updated;
        });
    }, [selectedCategory]);

    // 저장
    const handleSave = useCallback(() => {
        if (saveCategoryMappings(mappings)) {
            setMessage('저장되었습니다.');
            setTimeout(() => setMessage(''), 3000);
        } else {
            setMessage('저장에 실패했습니다.');
        }
    }, [mappings]);

    // Enter 키 처리
    const handleKeyPress = useCallback((e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            handleAddKeyword();
        }
    }, [handleAddKeyword]);

    const currentKeywords = selectedCategory ? (mappings[selectedCategory] || []) : [];

    return (
        <Card>
            <Card.Header>
                카테고리 매핑 관리
                <Button variant="success" className="btn-xs float-end" onClick={handleSave}>
                    <FontAwesomeIcon icon={faSave}/> 저장
                </Button>
            </Card.Header>
            <Card.Body>
                {message && (
                    <div className="alert alert-success py-1 mb-2">{message}</div>
                )}
                <Row>
                    <Col md={4}>
                        <Card>
                            <Card.Header className="py-1">디렉토리 목록</Card.Header>
                            <ListGroup variant="flush" style={{maxHeight: '400px', overflowY: 'auto'}}>
                                {categoryList?.map(category => (
                                    <ListGroup.Item
                                        key={category}
                                        action
                                        active={selectedCategory === category}
                                        onClick={() => handleCategorySelect(category)}
                                        className="py-1 d-flex justify-content-between align-items-center"
                                    >
                                        <span style={{fontSize: '0.85rem'}}>{category}</span>
                                        {mappings[category]?.length > 0 && (
                                            <Badge bg="secondary" pill>{mappings[category].length}</Badge>
                                        )}
                                    </ListGroup.Item>
                                ))}
                            </ListGroup>
                        </Card>
                    </Col>
                    <Col md={8}>
                        {selectedCategory ? (
                            <Card>
                                <Card.Header className="py-1">
                                    <strong>{selectedCategory}</strong> 키워드
                                </Card.Header>
                                <Card.Body>
                                    <InputGroup className="mb-2">
                                        <Form.Control
                                            type="text"
                                            placeholder="새 키워드 입력"
                                            value={newKeyword}
                                            onChange={(e) => setNewKeyword(e.target.value)}
                                            onKeyPress={handleKeyPress}
                                        />
                                        <Button variant="outline-primary" onClick={handleAddKeyword}>
                                            <FontAwesomeIcon icon={faPlus}/> 추가
                                        </Button>
                                    </InputGroup>
                                    <div className="d-flex flex-wrap gap-1">
                                        {currentKeywords.map(keyword => (
                                            <Badge
                                                key={keyword}
                                                bg="info"
                                                className="d-flex align-items-center gap-1"
                                                style={{fontSize: '0.85rem', padding: '0.4rem 0.6rem'}}
                                            >
                                                {keyword}
                                                <FontAwesomeIcon
                                                    icon={faTrash}
                                                    style={{cursor: 'pointer', marginLeft: '4px'}}
                                                    onClick={() => handleRemoveKeyword(keyword)}
                                                />
                                            </Badge>
                                        ))}
                                        {currentKeywords.length === 0 && (
                                            <span className="text-muted">등록된 키워드가 없습니다.</span>
                                        )}
                                    </div>
                                </Card.Body>
                            </Card>
                        ) : (
                            <div className="text-muted p-3">왼쪽에서 디렉토리를 선택하세요.</div>
                        )}
                    </Col>
                </Row>
                <div className="mt-3 text-muted" style={{fontSize: '0.8rem'}}>
                    <strong>사용법:</strong> 각 디렉토리에 서점 카테고리와 매칭될 키워드를 등록합니다.<br/>
                    예: "4_심리학뇌과학" → "심리학", "뇌과학", "인지과학" 등
                </div>
            </Card.Body>
        </Card>
    );
}

CategoryMapping.propTypes = {
    categoryList: PropTypes.array.isRequired,
};
