// Case: Arbitrary code execution via eval() on user input.
'use strict';

function buildFilter(expression) {
  // BUG: the caller-supplied expression is passed straight to eval(), so any
  // user-controlled value becomes running JavaScript (e.g. stealing cookies
  // via fetch, or crashing the process).
  const predicate = eval('(' + expression + ')');
  return (item) => predicate(item);
}

module.exports = { buildFilter };
