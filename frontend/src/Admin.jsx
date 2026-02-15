import {useOutletContext} from 'react-router-dom';

import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import CategoryAdmin from './CategoryAdmin';

export default function Admin() {
    useOutletContext();

    return (
        <>
            <CategoryAdmin contentType="book" title="책 카테고리 관리" />
            <CategoryAdmin contentType="comic" title="만화 카테고리 관리" />
        </>
    );
}
