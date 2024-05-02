import {useEffect, useRef, useState} from "react";
import PropTypes from 'prop-types';
import {getApiUrlPrefix} from "./Common";

export default function ViewHTML(props) {
    const ref = useRef(null);
    const [iframeHeight, setIframeHeight] = useState(0);
    const [url, setUrl] = useState("");

    useEffect(() => {
        console.log(`ViewHTML: useEffect(${props})`, props);
        setIframeHeight(document.body.scrollHeight);

        const url = getApiUrlPrefix() + '/download/' + props.bookId;
        console.log(url);
        setUrl(url);
        console.log(ref.current);

        return () => {
            setUrl('')
            setIframeHeight(0)
        };
    }, [props]);

    return (
        <iframe src={url} ref={ref} style={{display: 'block', width: '100%', height: iframeHeight, overflow: 'visible'}}/>
    );
}

ViewHTML.propTypes = {
    bookId: PropTypes.number.isRequired,
};


