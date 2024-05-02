import {useEffect, useState} from "react";
import PropTypes from 'prop-types';

import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Button} from 'react-bootstrap';

export default function Actions(props) {
    const [yes24SearchUrl, setYes24SearchUrl] = useState('');
    const [googleSearchUrl, setGoogleSearchUrl] = useState('');
    const [naverShoppingSearchUrl, setNaverShoppingSearchUrl] = useState('');
    const [naverSeriesSearchUrl, setNaverSeriesSearchUrl] = useState('');
    const [munpiaSearchUrl, setMunpiaSearchUrl] = useState('');
    const [ridiSearchUrl, setRidiSearchUrl] = useState('');

    const links = [
        {url: yes24SearchUrl, label: "Yes24"},
        {url: googleSearchUrl, label: "구글"},
        {url: naverShoppingSearchUrl, label: "네이버쇼핑"},
        {url: naverSeriesSearchUrl, label: "네이버시리즈"},
        {url: munpiaSearchUrl, label: "문피아"},
        {url: ridiSearchUrl, label: "RIDI"}
    ];

    useEffect(() => {
        // determine external search keyword from author and title
        const extractedWords = (props.bookInfo['author'] + ' ' + props.bookInfo['title']).match(/[\uAC00-\uD7A3a-zA-Z0-9]+/g);
        const keyword = extractedWords ? extractedWords.join(' ') : '';
        setYes24SearchUrl(`https://www.yes24.com/Product/Search?domain=ALL&query=${encodeURIComponent(keyword)}`);
        setGoogleSearchUrl(`https://www.google.com/search?sourceid=chrome&ie=UTF-8&oq=${encodeURIComponent(keyword)}&q=${encodeURIComponent(keyword)}&sourceid=chrome&ie=UTF-8`);
        setNaverShoppingSearchUrl(`https://search.shopping.naver.com/book/search?bookTabType=ALL&pageIndex=1&pageSize=40&sort=REL&query=${encodeURIComponent(keyword)}`);
        setNaverSeriesSearchUrl(`https://series.naver.com/search/search.series?t=all&fs=novel&q=${encodeURIComponent(keyword)}`);
        setMunpiaSearchUrl(`https://novel.munpia.com/page/hd.platinum/view/search/keyword/${encodeURIComponent(keyword)}/order/search_result`);
        setRidiSearchUrl(`https://ridibooks.com/search?adult_exclude=n&q=${encodeURIComponent(keyword)}`);
    }, [props]);

    return (
        <>
            <Button variant="outline-success" size="sm" onClick={props.toNextEntryClicked}>다음 책으로</Button>
            {links.map(link =>
                    link.url && (
                        <a key={link.label} href={link.url} target="_blank" rel="noreferrer">
                            <Button variant="outline-primary" size="sm">{link.label}</Button>
                        </a>
                    )
            )}
        </>
    );
}

Actions.propTypes = {
    toNextEntryClicked: PropTypes.func.isRequired,
    bookInfo: PropTypes.object.isRequired,
};