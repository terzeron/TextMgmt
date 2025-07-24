import {useEffect, useRef, useState} from "react";
import PropTypes from 'prop-types';
import {getApiUrlPrefix} from "./Common";

export default function ViewDOC(props) {
    const [url, setUrl] = useState("");
    const [iframeHeight, setIframeHeight] = useState(0);
    const ref = useRef(null);

    useEffect(() => {
        if (props.bookId && props.filePath) {
            setIframeHeight(document.body.scrollHeight);
            const uri = getApiUrlPrefix() + '/download/' + props.bookId + '/' + props.filePath;
            const wordViewerUrlPrefix = 'https://view.officeapps.live.com/op/embed.aspx?src=';
            setUrl(wordViewerUrlPrefix + encodeURIComponent(uri));
        }
    }, [props.bookId, props.filePath]);

    return (
        url && <iframe src={url} ref={ref} style={{display: 'block', width: '100%', height: iframeHeight, overflow: 'visible'}}/>
    );
}

ViewDOC.propTypes = {
    bookId: PropTypes.number.isRequired,
    filePath: PropTypes.string.isRequired
};
