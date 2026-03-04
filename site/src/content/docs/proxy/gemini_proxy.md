---
title: Gemini proxing demo
description: This is a demo of using CAPSEM to proxy requests to Gemini via the CAPSEM
  Proxy. It shows how to set up a simple agent that uses Gemini as its LLM, and how
  to enforce policies on the agent's behavior.
---
This is a demo of using CAPSEM to proxy requests to Gemini via the CAPSEM Proxy. It shows how to set up a simple agent that uses Gemini as its LLM, and how to enforce policies on the agent's behavior.


```python
import time
from google import genai
from google.genai import types

```

## Instantiate a proxied Gemini client

To proxy the requests to Gemini throught CAPSEM, we need to  pass the the CAPSEM Proxy URL as part of the HTTP options when creating the Gemini client.

We start by defining the http options with the CAPSEM Proxy URL as `base_url` and then instantiate the Gemini client with these options.


```python
CAPSEM_PROXY = "http://127.0.0.1:8000"
http_options = types.HttpOptions(base_url=CAPSEM_PROXY)
client = genai.Client(http_options=http_options)
```

## Calling Gemini via CAPSEM Proxy 

We can now use the Gemini client as usual, and all requests will be proxied through CAPSEM. This allows us to enforce policies on the requests and responses, such as filtering out sensitive information or input sanitization. Here is a simple example of generating content with Gemini via CAPSEM Proxy.




```python
def weather_function(location: str) -> dict:
    # Dummy implementation for illustration
    return {"temperature": "20°C", "location": location}

config = types.GenerateContentConfig(tools=[weather_function])

response = client.models.generate_content(
    model="gemini-2.5-flash",
    contents="what is the weather in paris?",
    config=config,
)
print(response.text)
```

    The weather in Paris is 20°C.


## Example of detections

### PII detection in tool call

In this example, the agent calls a tool that have PII
information in one of the argument. CAPSEM proxy detects 
the PII in the tool call and blocks the request.


```python
def contact_lookup(email: str) -> dict:
    # Dummy implementation for illustration
    return {"name": "Elie", "Company": "Google"}

config = types.GenerateContentConfig(tools=[contact_lookup])

response = client.models.generate_content(
    model="gemini-2.5-flash",
    contents="lookup a made-up test email to see if the contact lookup works",
    config=config,
)
print(response.text)

```

### PII detection in tool result

In this example, the agent calls a tool that returns PII information. CAPSEM proxy detects the PII in the tool result and blocks the response.


```python
def weather_pii(location: str) -> dict:
    # Dummy implementation for illustration
    return {"temperature": "20°C", "location": "PII test@gmail.com"}

config = types.GenerateContentConfig(tools=[weather_pii])

response = client.models.generate_content(
    model="gemini-2.5-flash",
    contents="what is the weather in paris?",
    config=config,
)
print(response.text)
```

### PII detection in model response
In this example, the model generates a response that contains PII information. CAPSEM proxy detects the PII in the response and blocks it.


```python
response = client.models.generate_content(
    model="gemini-2.5-flash",
    contents="My name is Elie - generate me idea for an email address @gmail.com",
)
print(response.text)
```


    ---------------------------------------------------------------------------

    ClientError                               Traceback (most recent call last)

    Cell In[20], line 1
    ----> 1 response = client.models.generate_content(
          2     model="gemini-2.5-flash",
          3     contents="My name is Elie - generate me idea for an email address @gmail.com",
          4 )
          5 print(response.text)


    File ~/git/capsem/.venv/lib/python3.13/site-packages/google/genai/models.py:5001, in Models.generate_content(self, model, contents, config)
       4999 while remaining_remote_calls_afc > 0:
       5000   i += 1
    -> 5001   response = self._generate_content(
       5002       model=model, contents=contents, config=parsed_config
       5003   )
       5005   function_map = _extra_utils.get_function_map(parsed_config)
       5006   if not function_map:


    File ~/git/capsem/.venv/lib/python3.13/site-packages/google/genai/models.py:3813, in Models._generate_content(self, model, contents, config)
       3810 request_dict = _common.convert_to_dict(request_dict)
       3811 request_dict = _common.encode_unserializable_types(request_dict)
    -> 3813 response = self._api_client.request(
       3814     'post', path, request_dict, http_options
       3815 )
       3817 if config is not None and getattr(
       3818     config, 'should_return_http_response', None
       3819 ):
       3820   return_value = types.GenerateContentResponse(sdk_http_response=response)


    File ~/git/capsem/.venv/lib/python3.13/site-packages/google/genai/_api_client.py:1292, in BaseApiClient.request(self, http_method, path, request_dict, http_options)
       1282 def request(
       1283     self,
       1284     http_method: str,
       (...)   1287     http_options: Optional[HttpOptionsOrDict] = None,
       1288 ) -> SdkHttpResponse:
       1289   http_request = self._build_request(
       1290       http_method, path, request_dict, http_options
       1291   )
    -> 1292   response = self._request(http_request, http_options, stream=False)
       1293   response_body = (
       1294       response.response_stream[0] if response.response_stream else ''
       1295   )
       1296   return SdkHttpResponse(headers=response.headers, body=response_body)


    File ~/git/capsem/.venv/lib/python3.13/site-packages/google/genai/_api_client.py:1128, in BaseApiClient._request(self, http_request, http_options, stream)
       1125     retry = tenacity.Retrying(**retry_kwargs)
       1126     return retry(self._request_once, http_request, stream)  # type: ignore[no-any-return]
    -> 1128 return self._retry(self._request_once, http_request, stream)


    File ~/git/capsem/.venv/lib/python3.13/site-packages/tenacity/__init__.py:477, in Retrying.__call__(self, fn, *args, **kwargs)
        475 retry_state = RetryCallState(retry_object=self, fn=fn, args=args, kwargs=kwargs)
        476 while True:
    --> 477     do = self.iter(retry_state=retry_state)
        478     if isinstance(do, DoAttempt):
        479         try:


    File ~/git/capsem/.venv/lib/python3.13/site-packages/tenacity/__init__.py:378, in BaseRetrying.iter(self, retry_state)
        376 result = None
        377 for action in self.iter_state.actions:
    --> 378     result = action(retry_state)
        379 return result


    File ~/git/capsem/.venv/lib/python3.13/site-packages/tenacity/__init__.py:420, in BaseRetrying._post_stop_check_actions.<locals>.exc_check(rs)
        418 retry_exc = self.retry_error_cls(fut)
        419 if self.reraise:
    --> 420     raise retry_exc.reraise()
        421 raise retry_exc from fut.exception()


    File ~/git/capsem/.venv/lib/python3.13/site-packages/tenacity/__init__.py:187, in RetryError.reraise(self)
        185 def reraise(self) -> t.NoReturn:
        186     if self.last_attempt.failed:
    --> 187         raise self.last_attempt.result()
        188     raise self


    File ~/.local/share/uv/python/cpython-3.13.1-macos-aarch64-none/lib/python3.13/concurrent/futures/_base.py:449, in Future.result(self, timeout)
        447     raise CancelledError()
        448 elif self._state == FINISHED:
    --> 449     return self.__get_result()
        451 self._condition.wait(timeout)
        453 if self._state in [CANCELLED, CANCELLED_AND_NOTIFIED]:


    File ~/.local/share/uv/python/cpython-3.13.1-macos-aarch64-none/lib/python3.13/concurrent/futures/_base.py:401, in Future.__get_result(self)
        399 if self._exception:
        400     try:
    --> 401         raise self._exception
        402     finally:
        403         # Break a reference cycle with the exception in self._exception
        404         self = None


    File ~/git/capsem/.venv/lib/python3.13/site-packages/tenacity/__init__.py:480, in Retrying.__call__(self, fn, *args, **kwargs)
        478 if isinstance(do, DoAttempt):
        479     try:
    --> 480         result = fn(*args, **kwargs)
        481     except BaseException:  # noqa: B902
        482         retry_state.set_exception(sys.exc_info())  # type: ignore[arg-type]


    File ~/git/capsem/.venv/lib/python3.13/site-packages/google/genai/_api_client.py:1105, in BaseApiClient._request_once(self, http_request, stream)
       1097 else:
       1098   response = self._httpx_client.request(
       1099       method=http_request.method,
       1100       url=http_request.url,
       (...)   1103       timeout=http_request.timeout,
       1104   )
    -> 1105   errors.APIError.raise_for_response(response)
       1106   return HttpResponse(
       1107       response.headers, response if stream else [response.text]
       1108   )


    File ~/git/capsem/.venv/lib/python3.13/site-packages/google/genai/errors.py:108, in APIError.raise_for_response(cls, response)
        106 status_code = response.status_code
        107 if 400 <= status_code < 500:
    --> 108   raise ClientError(status_code, response_json, response)
        109 elif 500 <= status_code < 600:
        110   raise ServerError(status_code, response_json, response)


    ClientError: 403 None. {'detail': 'Request blocked by security policy: PII detected in model response: EMAIL_ADDRESS(count=11, score=1.00, action=BLOCK), PERSON(count=3, score=0.85, action=LOG)'}

