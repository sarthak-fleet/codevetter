// Case: Reflected XSS via dangerouslySetInnerHTML in a React component.
import React from 'react';

interface CommentProps {
  body: string; // user-supplied comment markdown/html
}

export const Comment: React.FC<CommentProps> = ({ body }) => {
  // BUG: raw user-supplied content is rendered as HTML without sanitization.
  // An attacker can inject <script> or event-handler payloads that execute in
  // every viewer's session.
  return <div dangerouslySetInnerHTML={{ __html: body }} />;
};
