<?php
namespace Foo;

class ComplexDefaults
{
    public function __construct(
        \Foo\Service $service,
        array $config = ['key' => 'value', 'nested' => ['a', 'b']],
        string $name = self::DEFAULT_NAME,
        ?\Foo\Logger $logger = null,
        int $timeout = 30 * 60
    ) {}

    const DEFAULT_NAME = 'default';
}
