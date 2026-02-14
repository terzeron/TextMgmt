import {useEffect, useState, useCallback} from 'react';
import {useOutletContext} from 'react-router-dom';

import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {jsonGetReq} from './Common';
import CategoryMapping from './CategoryMapping';
import CategoryMismatch from './CategoryMismatch';

export default function Admin() {
    const {searchResults, hasSearched} = useOutletContext();
    const [categoryList, setCategoryList] = useState([]);
    const [comicsCategoryList, setComicsCategoryList] = useState([]);
    const [errorMessage, setErrorMessage] = useState('');

    const loadBookCategories = useCallback(() => {
        jsonGetReq('/categories', null, (categoryCounts) => {
            const categoryList = Object.keys(categoryCounts);
            const filtered = categoryList.filter(c => c !== '_root' && !c.includes('/')).sort((a, b) => a.localeCompare(b));
            setCategoryList(filtered);
        }, (error) => {
            setErrorMessage(`카테고리 목록을 불러올 수 없습니다. ${error}`);
        });
    }, []);

    const loadComicsCategories = useCallback(() => {
        jsonGetReq('/comics/categories', null, (categoryCounts) => {
            const categoryList = Object.keys(categoryCounts);
            const filtered = categoryList.filter(c => c !== '_root' && !c.includes('/')).sort((a, b) => a.localeCompare(b));
            setComicsCategoryList(filtered);
        }, (error) => {
            setErrorMessage(`만화 카테고리 목록을 불러올 수 없습니다. ${error}`);
        });
    }, []);

    useEffect(() => {
        loadBookCategories();
        loadComicsCategories();
    }, [loadBookCategories, loadComicsCategories]);

    return (
        <>
            {errorMessage && (
                <div className="alert alert-danger">{errorMessage}</div>
            )}
            <CategoryMapping categoryList={categoryList} contentType="book" title="책 카테고리 관리" onCategoryChanged={loadBookCategories}/>
            <CategoryMapping categoryList={comicsCategoryList} contentType="comic" title="만화 카테고리 관리" onCategoryChanged={loadComicsCategories}/>
            <CategoryMismatch/>
        </>
    );
}
