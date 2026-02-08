import {useEffect, useState} from 'react';
import {useOutletContext} from 'react-router-dom';

import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {jsonGetReq} from './Common';
import CategoryMapping from './CategoryMapping';
import CategoryMismatch from './CategoryMismatch';

export default function Admin() {
    const {searchResults, hasSearched} = useOutletContext();
    const [categoryList, setCategoryList] = useState([]);
    const [errorMessage, setErrorMessage] = useState('');

    useEffect(() => {
        const categoryListUrl = '/categories';
        jsonGetReq(categoryListUrl, null, (categoryCounts) => {
            // _root 제외하고 정렬
            const categoryList = Object.keys(categoryCounts);
            const filtered = categoryList.filter(c => c !== '_root' && !c.includes('/')).sort((a, b) => a.localeCompare(b));
            setCategoryList(filtered);
        }, (error) => {
            setErrorMessage(`카테고리 목록을 불러올 수 없습니다. ${error}`);
        });
    }, []);

    return (
        <>
            {errorMessage && (
                <div className="alert alert-danger">{errorMessage}</div>
            )}
            <CategoryMapping categoryList={categoryList}/>
            <CategoryMismatch/>
        </>
    );
}
