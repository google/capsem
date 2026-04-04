const http = require('http');
const { chromium } = require('playwright');

let browser = null;
let page = null;

async function initBrowser() {
  if (!browser) {
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage();
  }
}

async function handleAction(action, params) {
  await initBrowser();

  switch (action) {
    case 'navigate': {
      const { url, timeout = 30000, waitUntil = 'load' } = params;
      const response = await page.goto(url, { timeout, waitUntil });
      return {
        url: response.url(),
        title: await page.title(),
        status: response.status(),
      };
    }

    case 'click': {
      const { selector, timeout = 5000 } = params;
      await page.waitForSelector(selector, { timeout });
      await page.click(selector);
      return { clicked: selector };
    }

    case 'type': {
      const { selector, text, clearFirst = true } = params;
      if (clearFirst) {
        await page.fill(selector, '');
      }
      await page.type(selector, text);
      return { typed: text, into: selector };
    }

    case 'screenshot': {
      const { selector, fullPage = false, maxWidth = 1280 } = params;
      let screenshot;
      if (selector) {
        const element = await page.$(selector);
        if (!element) throw new Error(`Element not found: ${selector}`);
        screenshot = await element.screenshot({ type: 'png' });
      } else {
        screenshot = await page.screenshot({ fullPage, type: 'png' });
      }
      return {
        image: screenshot.toString('base64'),
        width: maxWidth,
        format: 'png',
      };
    }

    case 'evaluate': {
      const { javascript, timeout = 10000 } = params;
      const result = await page.evaluate(javascript);
      return { result };
    }

    case 'getText': {
      const { selector, maxLength = 5000 } = params;
      const elements = await page.$$(selector);
      const texts = await Promise.all(
        elements.map(async (el) => {
          const text = await el.textContent();
          return text || '';
        })
      );
      const combined = texts.filter(t => t).join('\n\n');
      return {
        text: combined.substring(0, maxLength),
        length: combined.length,
        elementsFound: elements.length,
      };
    }

    case 'fillForm': {
      const { fields } = params;
      const results = [];
      for (const field of fields) {
        await page.fill(field.selector, field.value);
        results.push({ selector: field.selector, value: field.value });
      }
      return { filled: results };
    }

    case 'getContent': {
      const { selector, format = 'text', maxLength = 5000 } = params;
      let content;
      if (selector) {
        const element = await page.$(selector);
        if (!element) throw new Error(`Element not found: ${selector}`);
        if (format === 'html') {
          content = await element.innerHTML();
        } else {
          content = await element.innerText();
        }
      } else {
        if (format === 'html') {
          content = await page.content();
        } else {
          content = await page.innerText('body');
        }
      }
      return {
        content: content.substring(0, maxLength),
        length: content.length,
        format,
      };
    }

    case 'close': {
      if (browser) {
        await browser.close();
        browser = null;
        page = null;
      }
      return { closed: true };
    }

    default:
      throw new Error(`Unknown action: ${action}`);
  }
}

const server = http.createServer(async (req, res) => {
  if (req.method === 'POST' && req.url === '/execute') {
    let body = '';
    req.on('data', chunk => { body += chunk.toString(); });
    req.on('end', async () => {
      try {
        const { action, params } = JSON.parse(body);
        const result = await handleAction(action, params);
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ result }));
      } catch (error) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: error.message }));
      }
    });
  } else {
    res.writeHead(404);
    res.end();
  }
});

const PORT = 0; // Let OS assign a random available port
server.listen(PORT, '127.0.0.1', () => {
  const address = server.address();
  // Output the endpoint URL for the Rust side to parse
  console.log(`ws://127.0.0.1:${address.port}`);
});
