<?php
namespace Magento\Framework;

class DataObject
{
    public function __construct(
        ?\Magento\Framework\App\RequestInterface $request = null,
        string|int|null $id = null,
        \Foo\Bar|\Foo\Baz $service
    ) {}
}
