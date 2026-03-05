<?php
namespace Magento\Framework\App;

class Service
{
    public function __construct(
        \Magento\Framework\App\Config $config
    ) {}

    public function getData(): array
    {
        return [];
    }

    public static function create(): static
    {
        return new static();
    }

    final public function getVersion(): string
    {
        return '1.0';
    }

    private function internalMethod(): void {}

    protected function protectedMethod(): void {}

    public function processRequest(
        \Magento\Framework\App\RequestInterface $request,
        ?string $format = null
    ): \Magento\Framework\App\ResponseInterface {
        return new \Magento\Framework\App\Response\Http();
    }
}
