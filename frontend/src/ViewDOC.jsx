import {useEffect, useRef, useState} from "react";
import PropTypes from 'prop-types';
import {getApiUrlPrefix} from "./Common";

export default function ViewDOC(props) {
    const [url, setUrl] = useState("");
    const [iframeHeight, setIframeHeight] = useState(0);
    const ref = useRef(null);

    useEffect(() => {
        console.log(`ViewDOC: useEffect()`, props);
        setIframeHeight(document.body.scrollHeight);
        const uri = getApiUrlPrefix() + '/download/' + props.bookId;
        const wordViewerUrlPrefix = 'https://view.officeapps.live.com/op/embed.aspx?src=';
        setUrl(wordViewerUrlPrefix + encodeURIComponent(uri));
        console.log(wordViewerUrlPrefix + encodeURIComponent(uri));
    }, [props]);

    return (
        url && <iframe src={url} ref={ref} style={{display: 'block', width: '100%', height: iframeHeight, overflow: 'visible'}}/>
    );
}

ViewDOC.propTypes = {
    bookId: PropTypes.number.isRequired
};
