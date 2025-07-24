import {useEffect, useState} from "react";
import PropTypes from 'prop-types';
import {getApiUrlPrefix} from "./Common";

export default function ViewImage(props) {
  const [url, setUrl] = useState("");

  useEffect(() => {
    if (props && props.bookId && props.filePath) {
      const url = getApiUrlPrefix() + '/download/' + props.bookId + '/' + props.filePath;
      setUrl(url);
    }

    return () => {
      setUrl('')
    };
  }, [props.bookId, props.filePath]);

  return (
    <img src={url} alt='book image'/>
  );
}

ViewImage.propTypes = {
    bookId: PropTypes.number.isRequired,
    filePath: PropTypes.string.isRequired
}



