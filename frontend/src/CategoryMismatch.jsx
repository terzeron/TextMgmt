import {useEffect, useState, useCallback, useMemo} from 'react';

import 'bootstrap/dist/css/bootstrap.min.css';
import {Button, Card, Row, Col, Spinner} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faChevronDown, faChevronRight} from '@fortawesome/free-solid-svg-icons';

import {RichTreeView} from '@mui/x-tree-view/RichTreeView';

import {jsonGetReq, jsonDeleteReq, jsonPostReq} from './Common';
import {CustomTreeItem} from './Folder';
import './Folder.css';
import {findCommonPrefix, buildFolderHierarchy, findFolderInTree, updateFolderInTree} from './folderUtils';

function buildMismatchCounts(mismatchData) {
    const counts = {};
    for (const item of mismatchData.mismatches || []) {
        counts[item.category] = Math.abs(item.diff);
    }
    for (const item of mismatchData.es_only || []) {
        counts[item.category] = item.es_count;
    }
    for (const item of mismatchData.fs_only || []) {
        counts[item.category] = item.fs_count;
    }
    return counts;
}

export default function CategoryMismatch() {
    const [folderData, setFolderData] = useState([]);

    const [loading, setLoading] = useState(false);
    const [error, setError] = useState('');
    const [isOpen, setIsOpen] = useState(false);
    const [expandedItems, setExpandedItems] = useState([]);
    const [selectedMismatch, setSelectedMismatch] = useState(null);
    const [actionResult, setActionResult] = useState(null);

    const onItemClick = useCallback((selectedId) => {
        const selectedFolderData = findFolderInTree(folderData, selectedId);
        if (!selectedFolderData || selectedFolderData.fileType !== 'folder') return;
        if (selectedFolderData.isVirtualParent) return;
        if (!selectedFolderData.count) return;
        if (selectedFolderData.booksLoaded) return;

        jsonGetReq('/category-mismatches/' + selectedId, null, (result) => {
            const entries = [];

            // ES에만 있는 항목 (FS에서 삭제됨)
            for (const item of result.es_only || []) {
                entries.push({
                    id: selectedId + '/es_' + item.book_id.toString(),
                    label: item.title + '.' + item.file_type,
                    fileType: item.file_type,
                    children: [],
                    mismatchType: 'es_only',
                    bookId: item.book_id,
                    category: selectedId,
                    filePath: item.file_path,
                });
            }

            // FS에만 있는 항목 (ES에서 삭제됨)
            for (const item of result.fs_only || []) {
                entries.push({
                    id: selectedId + '/fs_' + item.file_name,
                    label: item.file_name,
                    fileType: 'unknown',
                    children: [],
                    mismatchType: 'fs_only',
                    category: selectedId,
                    filePath: item.file_path,
                });
            }

            const data = updateFolderInTree(folderData, selectedId, (folder) => {
                const existingSubfolders = (folder.children || []).filter(c => c.fileType === 'folder');
                return {
                    ...folder,
                    booksLoaded: true,
                    children: [...existingSubfolders, ...entries],
                };
            });
            setFolderData(data);
        });
    }, [folderData]);

    const handleDeleteEsDoc = useCallback(() => {
        if (!selectedMismatch || selectedMismatch.mismatchType !== 'es_only') return;
        setActionResult(null);
        jsonDeleteReq('/books/' + selectedMismatch.bookId, null,
            (result) => {
                const warning = result?.warning;
                const msg = warning ? `책 정보가 삭제되었습니다. (${warning})` : '책 정보가 삭제되었습니다.';
                setActionResult({type: 'success', message: msg});
                const data = updateFolderInTree(folderData, selectedMismatch.category, (folder) => ({
                    ...folder,
                    children: (folder.children || []).filter(c => c.id !== selectedMismatch.id),
                }));
                setFolderData(data);

                setSelectedMismatch(null);
            },
            (error) => {
                setActionResult({type: 'error', message: `삭제 실패: ${error}`});
            }
        );
    }, [selectedMismatch, folderData]);

    const handleIndexFile = useCallback(() => {
        if (!selectedMismatch || selectedMismatch.mismatchType !== 'fs_only') return;
        setActionResult(null);
        jsonPostReq('/category-mismatches/index-file', {file_path: selectedMismatch.filePath},
            (result) => {
                setActionResult({type: 'success', message: 'ES에 적재되었습니다.'});
                const data = updateFolderInTree(folderData, selectedMismatch.category, (folder) => ({
                    ...folder,
                    children: (folder.children || []).filter(c => c.id !== selectedMismatch.id),
                }));
                setFolderData(data);

                setSelectedMismatch(null);
            },
            (error) => {
                setActionResult({type: 'error', message: `ES 적재 실패: ${error}`});
            }
        );
    }, [selectedMismatch, folderData]);

    const handleDeleteFile = useCallback(() => {
        if (!selectedMismatch || selectedMismatch.mismatchType !== 'fs_only') return;
        setActionResult(null);
        jsonPostReq('/category-mismatches/delete-file', {file_path: selectedMismatch.filePath},
            (result) => {
                setActionResult({type: 'success', message: '파일이 삭제되었습니다.'});
                const data = updateFolderInTree(folderData, selectedMismatch.category, (folder) => ({
                    ...folder,
                    children: (folder.children || []).filter(c => c.id !== selectedMismatch.id),
                }));
                setFolderData(data);

                setSelectedMismatch(null);
            },
            (error) => {
                setActionResult({type: 'error', message: `파일 삭제 실패: ${error}`});
            }
        );
    }, [selectedMismatch, folderData]);

    const treeViewStyles = useMemo(() => ({
        height: 'fit-content',
        flexGrow: 1,
        overflowY: 'auto',
    }), []);

    const loadData = useCallback(() => {
        setLoading(true);
        setError('');

        let categoriesResult = null;
        let mismatchResult = null;
        let completed = 0;
        let hasError = false;

        const tryBuild = () => {
            completed++;
            if (completed < 2 || hasError) return;

            const mismatchCounts = buildMismatchCounts(mismatchResult);

            // 불일치가 있는 카테고리만 표시
            const esCategories = Object.keys(categoriesResult).filter(c => c !== '_root');
            const fsOnlyCategories = (mismatchResult.fs_only || []).map(item => item.category);
            const allCategories = [...new Set([...esCategories, ...fsOnlyCategories])]
                .filter(cat => mismatchCounts[cat] > 0)
                .sort((a, b) => a.localeCompare(b));

            const commonPrefix = findCommonPrefix(allCategories);
            const data = buildFolderHierarchy(allCategories, commonPrefix, mismatchCounts);

            setFolderData(data);
            setExpandedItems([]);
            setLoading(false);
        };

        jsonGetReq('/categories', null,
            (result) => { categoriesResult = result; tryBuild(); },
            (err) => { hasError = true; setError(`카테고리 목록을 불러올 수 없습니다. ${err}`); setLoading(false); }
        );

        jsonGetReq('/category-mismatches', null,
            (result) => { mismatchResult = result; tryBuild(); },
            (err) => { hasError = true; setError(`불일치 데이터를 불러올 수 없습니다. ${err}`); setLoading(false); }
        );
    }, []);

    useEffect(() => {
        loadData();
    }, [loadData]);

    if (!isOpen) {
        return (
            <Card className="mt-3">
                <Card.Header
                    onClick={() => setIsOpen(true)}
                    style={{cursor: 'pointer', userSelect: 'none'}}
                    className="py-2">
                    <FontAwesomeIcon icon={faChevronRight} className="me-2"/>
                    불일치 관리
                </Card.Header>
            </Card>
        );
    }

    return (
        <Card className="mt-3">
            <Card.Header
                onClick={() => setIsOpen(false)}
                style={{cursor: 'pointer', userSelect: 'none'}}
                className="py-2">
                <FontAwesomeIcon icon={faChevronDown} className="me-2"/>
                불일치 관리
            </Card.Header>
            <Card.Body>
                {error && (
                    <div className="alert alert-danger py-1 mb-2">{error}</div>
                )}
                {loading ? (
                    <div className="text-center p-4">
                        <Spinner animation="border"/>
                        <p className="mt-2">로딩 중...</p>
                    </div>
                ) : folderData.length === 0 ? (
                    <div className="text-muted p-3">불일치 없음</div>
                ) : (
                    <Row className="g-0">
                        <Col md={4}>
                            <Card>
                                <Card.Header className="py-1">디렉토리 목록</Card.Header>
                                <div id="dir_list">
                                    <RichTreeView
                                        items={folderData}
                                        aria-label="category mismatches"
                                        sx={treeViewStyles}
                                        slots={{item: CustomTreeItem}}
                                        expandedItems={expandedItems}
                                        onSelectedItemsChange={(event, selectedId) => {
                                            // 불일치 항목(leaf) 클릭 확인
                                            let foundItem = null;
                                            for (const folder of folderData) {
                                                for (const child of folder.children || []) {
                                                    if (child.id === selectedId && child.mismatchType) {
                                                        foundItem = child;
                                                        break;
                                                    }
                                                    for (const grandchild of child.children || []) {
                                                        if (grandchild.id === selectedId && grandchild.mismatchType) {
                                                            foundItem = grandchild;
                                                            break;
                                                        }
                                                    }
                                                    if (foundItem) break;
                                                }
                                                if (foundItem) break;
                                            }

                                            if (foundItem) {
                                                setSelectedMismatch(foundItem);
                                                setActionResult(null);
                                                return;
                                            }

                                            // 폴더 클릭 — 펼치기/접기
                                            setSelectedMismatch(null);
                                            setActionResult(null);
                                            const willExpand = !expandedItems.includes(selectedId);
                                            setExpandedItems((prev) =>
                                                willExpand
                                                    ? [...prev, selectedId]
                                                    : prev.filter(x => x !== selectedId)
                                            );
                                            if (willExpand) {
                                                onItemClick(selectedId);
                                            }
                                        }}
                                    />
                                </div>
                            </Card>
                        </Col>
                        <Col md={8}>
                            {selectedMismatch && (
                                <Card>
                                    <Card.Header className="py-1">
                                        <strong>{selectedMismatch.label}</strong>
                                    </Card.Header>
                                    <Card.Body>
                                        <div className="text-muted mb-2" style={{fontSize: '0.85rem'}}>
                                            {selectedMismatch.mismatchType === 'es_only'
                                                ? '책 정보만 존재하고 파일시스템에는 존재하지 않습니다.'
                                                : '책 정보는 없고 파일시스템에만 존재합니다.'}
                                        </div>
                                        <div className="d-flex flex-wrap gap-1">
                                            {selectedMismatch.mismatchType === 'es_only' && (
                                                <>
                                                    <Button
                                                        variant="outline-warning" size="sm"
                                                        onClick={() => window.open(`/edit/${selectedMismatch.bookId}?category=${encodeURIComponent(selectedMismatch.category)}`, '_blank', 'noopener')}
                                                    >
                                                        편집
                                                    </Button>
                                                    <Button
                                                        variant="outline-primary" size="sm"
                                                        onClick={() => window.open(`/view/${selectedMismatch.bookId}?category=${encodeURIComponent(selectedMismatch.category)}`, '_blank', 'noopener')}
                                                    >
                                                        조회
                                                    </Button>
                                                    <Button
                                                        variant="outline-danger" size="sm"
                                                        onClick={handleDeleteEsDoc}
                                                    >
                                                        삭제
                                                    </Button>
                                                </>
                                            )}
                                            {selectedMismatch.mismatchType === 'fs_only' && (
                                                <>
                                                    <Button
                                                        variant="outline-success" size="sm"
                                                        onClick={handleIndexFile}
                                                    >
                                                        ES 적재
                                                    </Button>
                                                    <Button
                                                        variant="outline-danger" size="sm"
                                                        onClick={handleDeleteFile}
                                                    >
                                                        파일 삭제
                                                    </Button>
                                                </>
                                            )}
                                        </div>
                                    </Card.Body>
                                </Card>
                            )}
                            {!selectedMismatch && !actionResult && (
                                <div className="text-muted p-3">왼쪽에서 불일치 항목을 선택하세요.</div>
                            )}
                            {actionResult && (
                                <div className={`p-3 ${actionResult.type === 'success' ? 'text-success' : 'text-danger'}`} style={{fontSize: '0.85rem'}}>
                                    {actionResult.message}
                                </div>
                            )}
                        </Col>
                    </Row>
                )}
            </Card.Body>
        </Card>
    );
}
